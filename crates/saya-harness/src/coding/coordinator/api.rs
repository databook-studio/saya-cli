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
    #[error("child did not settle within the cancellation join grace period")]
    Unsettled,
    #[error("child-owned resources did not settle")]
    Settlement(#[source] saya_agent::ToolError),
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
    run_child_inner(spec, factory, events, cancellation, None, None).await
}
/// Native child invocation with job-owned budgets and one active turn per agent.
/// The caller must retain and join this future through cancellation; dropping it
/// does not settle its durable reservations or join the cancellation monitor.
pub async fn run_child_budgeted(
    spec: ChildExecutionSpec,
    factory: &dyn ChildFactory,
    events: ChildEventSender,
    cancellation: CancellationToken,
    ledger: &JobBudgetLedger,
) -> Result<saya_agent::AgentOutput, ChildRunError> {
    run_child_controlled(spec, factory, events, cancellation, ledger, None).await
}

pub(super) async fn run_child_controlled(
    spec: ChildExecutionSpec,
    factory: &dyn ChildFactory,
    events: ChildEventSender,
    cancellation: CancellationToken,
    ledger: &JobBudgetLedger,
    mailbox: Option<saya_agent::AgentMailbox>,
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
    if let Some(reason) = cancellation.reason() {
        return Err(ChildRunError::Cancelled { reason });
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
    let mut invocation_guard =
        super::super::child::failure::InvocationGuard(Some(child_token.clone()));
    let caller_token = cancellation.clone();
    let ledger_token = ledger.cancellation_token();
    let monitor_token = child_token.clone();
    let deadline = ledger.remaining().await;
    let monitor = super::super::child::failure::AbortMonitor(Some(tokio::spawn(async move {
        tokio::select! {
            biased;
            _ = caller_token.cancelled() => { monitor_token.cancel_with_reason(caller_token.reason().unwrap_or(CancellationReason::User)); },
            _ = ledger_token.cancelled() => { monitor_token.cancel_with_reason(ledger_token.reason().unwrap_or(CancellationReason::OwnerFailure)); },
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
    })));
    let result = run_child_inner(spec, factory, events, child_token, Some(&view), mailbox).await;
    monitor.stop().await;
    let started = ledger.child_started(&agent_id, &invocation_id).await;
    ledger
        .finish_child(
            session,
            if matches!(
                &result,
                Err(ChildRunError::Settlement(_)
                    | ChildRunError::Agent(AgentError::EffectOwnerFailure(_)))
            ) {
                EffectOutcome::Unknown
            } else if started {
                EffectOutcome::Succeeded
            } else {
                EffectOutcome::NotStarted
            },
        )
        .await
        .map_err(|_| ChildRunError::OwnerFailure)?;
    invocation_guard.0 = None;
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
    mailbox: Option<saya_agent::AgentMailbox>,
) -> Result<saya_agent::AgentOutput, ChildRunError> {
    let authority = spec.authority();
    if &events.job_id != authority.job_id()
        || &events.agent_id != authority.agent_id()
        || &events.invocation_id != authority.invocation_id()
    {
        return Err(ChildRunError::ChannelIdentityMismatch);
    }
    if let Some(reason) = cancellation.reason() {
        return Err(ChildRunError::Cancelled { reason });
    }
    let ChildComponents {
        provider,
        tools,
        definitions,
        approval,
    } = factory.build(&spec, &cancellation)?;
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
    let control = match mailbox {
        Some(mailbox) => control.with_mailbox(mailbox),
        None => control,
    };
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
    tools
        .settle_invocation()
        .await
        .map_err(ChildRunError::Settlement)?;
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
