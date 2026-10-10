use super::events::{ChildEventSender, ChildEventSink};
use saya_agent::{
    AgentError, AgentRunControl, CancellationReason, CancellationToken, EffectAdmission,
    EffectAdmissionError, EffectOutcome, run_agent_with_control,
};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use thiserror::Error;

use super::super::{
    budget::{ChildCountLimits, JobBudgetLedger},
    child::{ChildComponents, ChildExecutionSpec, ChildFactory, ChildFactoryError},
};

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ChildRunError {
    #[error(transparent)]
    Factory(#[from] ChildFactoryError),
    #[error("child event channel identity does not match its execution spec")]
    ChannelIdentityMismatch,
    #[error("child progress owner failed")]
    OwnerFailure,
    #[error("shared child or effect budget denied admission")]
    BudgetDenied,
    #[error("child invocation was cancelled: {reason:?}")]
    Cancelled { reason: CancellationReason },
    #[error(transparent)]
    Agent(#[from] AgentError),
}

/// Run one child through the existing agent loop; scheduling and joining stay with the caller.
pub async fn run_child(
    spec: ChildExecutionSpec,
    factory: &dyn ChildFactory,
    events: ChildEventSender,
    cancellation: CancellationToken,
) -> Result<saya_agent::AgentOutput, ChildRunError> {
    run_child_inner(spec, factory, events, cancellation, None).await
}
pub async fn run_child_budgeted(
    spec: ChildExecutionSpec,
    factory: &dyn ChildFactory,
    events: ChildEventSender,
    cancellation: CancellationToken,
    ledger: &JobBudgetLedger,
) -> Result<saya_agent::AgentOutput, ChildRunError> {
    let (job_id, agent_id, invocation_id) = {
        let authority = spec.authority();
        (
            authority.job_id().clone(),
            authority.agent_id().clone(),
            authority.invocation_id().clone(),
        )
    };
    if job_id != ledger.job_id().await
        || events.job_id != job_id
        || events.agent_id != agent_id
        || events.invocation_id != invocation_id
    {
        return Err(ChildRunError::ChannelIdentityMismatch);
    }
    let limits = spec.answering_limits();
    let logical_requests = limits.max_requests.ok_or(ChildRunError::BudgetDenied)?;
    let tool_calls = limits.max_tool_calls.ok_or(ChildRunError::BudgetDenied)?;
    let session = ledger
        .begin_child(
            &agent_id,
            &invocation_id,
            ChildCountLimits {
                logical_requests,
                tool_calls,
            },
        )
        .await
        .map_err(map_admission_error)?;
    let view = session.view().clone();
    let child_token = CancellationToken::new();
    let caller_token = cancellation.clone();
    let ledger_token = ledger.cancellation_token();
    let monitor_token = child_token.clone();
    let deadline = ledger.remaining().await;
    let monitor = tokio::spawn(async move {
        tokio::select! {
            biased;
            _ = caller_token.cancelled() => monitor_token.cancel_with_reason(caller_token.reason().unwrap_or(CancellationReason::User)),
            _ = ledger_token.cancelled() => monitor_token.cancel_with_reason(ledger_token.reason().unwrap_or(CancellationReason::OwnerFailure)),
            _ = tokio::time::sleep(deadline) => {
                ledger_token.cancel_with_reason(CancellationReason::Budget(
                    saya_agent::BudgetDimension::ElapsedTime,
                ));
                let reason = ledger_token
                    .reason()
                    .unwrap_or(CancellationReason::Budget(
                        saya_agent::BudgetDimension::ElapsedTime,
                    ));
                monitor_token.cancel_with_reason(reason);
            }
        }
    });
    let result = run_child_inner(spec, factory, events, child_token, Some(&view)).await;
    monitor.abort();
    let _ = monitor.await;
    let started = ledger.child_started(&agent_id, &invocation_id).await;
    ledger
        .finish_child(
            session,
            if started {
                EffectOutcome::Succeeded
            } else {
                EffectOutcome::NotStarted
            },
        )
        .await
        .map_err(|_| ChildRunError::OwnerFailure)?;
    result
}

fn map_admission_error(error: EffectAdmissionError) -> ChildRunError {
    match error {
        EffectAdmissionError::Denied => ChildRunError::BudgetDenied,
        EffectAdmissionError::OwnerFailure(_)
        | EffectAdmissionError::UnsupportedPhysicalCoverage => ChildRunError::OwnerFailure,
        _ => ChildRunError::OwnerFailure,
    }
}
async fn run_child_inner(
    spec: ChildExecutionSpec,
    factory: &dyn ChildFactory,
    events: ChildEventSender,
    cancellation: CancellationToken,
    admission: Option<&dyn EffectAdmission>,
) -> Result<saya_agent::AgentOutput, ChildRunError> {
    let authority = spec.authority();
    if &events.job_id != authority.job_id()
        || &events.agent_id != authority.agent_id()
        || &events.invocation_id != authority.invocation_id()
    {
        return Err(ChildRunError::ChannelIdentityMismatch);
    }
    let ChildComponents {
        provider,
        tools,
        definitions,
        approval,
    } = factory.build(&spec)?;
    let sink = ChildEventSink {
        events,
        cancellation: cancellation.clone(),
        owner_failure: AtomicBool::new(false),
    };
    let (request, limits, answering, preflight) = spec.into_run_parts();
    let control = AgentRunControl::new(
        limits,
        answering,
        approval.as_ref(),
        &sink,
        cancellation.clone(),
    );
    let control = match preflight {
        Some(config) => control.with_request_preflight(config),
        None => control,
    };
    let control = match admission {
        Some(owner) => control.with_physical_effect_admission(owner),
        None => control,
    };
    let result = run_agent_with_control(
        provider.as_ref(),
        tools.as_ref(),
        request,
        definitions,
        control,
    )
    .await;
    let reason = cancellation.reason();
    if sink.owner_failure.load(Ordering::Acquire)
        || reason == Some(CancellationReason::OwnerFailure)
    {
        return Err(ChildRunError::OwnerFailure);
    }
    if result.is_ok() && reason == Some(CancellationReason::User) {
        return Err(ChildRunError::Cancelled {
            reason: CancellationReason::User,
        });
    }
    if matches!(&result, Err(AgentError::Cancelled)) {
        if let Some(reason) = reason {
            return Err(ChildRunError::Cancelled { reason });
        }
    }
    result.map_err(ChildRunError::Agent)
}
