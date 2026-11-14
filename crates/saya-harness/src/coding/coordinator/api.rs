use super::events::{ChildEventSender, ChildEventSink};
use saya_agent::{
    AgentError, AgentRunControl, CancellationReason, CancellationToken, ContextPreamble,
    EffectAdmission, EffectAdmissionError, EffectOutcome, run_agent_with_control,
};
use std::sync::atomic::{AtomicBool, Ordering};
use thiserror::Error;

use super::super::{
    budget::{ChildCountLimits, JobBudgetLedger},
    child::{
        ChildComponents, ChildExecutionSpec, ChildFactory, ChildFactoryError, OwnerFailureSnapshot,
    },
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
    /// A native owner reported a payload-free failure only after the tool
    /// invocation and its mandatory settlement completed. `prior` retains an
    /// earlier provider/user cause; the snapshot supplies no untrusted text.
    #[error("child native owner operation failed")]
    OwnerOperation {
        prior: Option<Box<ChildRunError>>,
        failure: OwnerFailureSnapshot,
    },
    #[error("child failed and cleanup is unconfirmed ({kind:?})")]
    Cleanup {
        primary: Box<ChildRunError>,
        kind: ChildCleanupFailure,
    },
    #[error("child progress delivery failed")]
    ProgressFailure,
    #[error("shared child or effect budget denied admission")]
    BudgetDenied,
    #[error("child invocation was cancelled: {reason:?}")]
    Cancelled { reason: CancellationReason },
    #[error(transparent)]
    Agent(#[from] AgentError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildCleanupFailure {
    Resources,
    BudgetPersistence,
    Progress,
}

/// Run one child through the existing agent loop; scheduling and joining stay with the caller.
pub async fn run_child(
    spec: ChildExecutionSpec,
    factory: &dyn ChildFactory,
    events: ChildEventSender,
    cancellation: CancellationToken,
) -> Result<saya_agent::AgentOutput, ChildRunError> {
    if spec.aggregate_uncapped() {
        return Err(ChildRunError::BudgetDenied);
    }
    let result = run_child_inner(spec, factory, events, cancellation, None, None).await;
    attach_owner_failure(factory, result)
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
    if spec.aggregate_uncapped() != ledger.is_uncapped().await {
        return Err(ChildRunError::BudgetDenied);
    }
    if let Some(reason) = cancellation.reason() {
        return Err(ChildRunError::Cancelled { reason });
    }
    let limits = spec.answering_limits();
    let session = if spec.aggregate_uncapped() {
        ledger.begin_child_uncapped(&agent_id, &invocation_id).await
    } else {
        let logical_requests = limits.max_requests.ok_or(ChildRunError::BudgetDenied)?;
        let tool_calls = limits.max_tool_calls.ok_or(ChildRunError::BudgetDenied)?;
        ledger
            .begin_child(
                &agent_id,
                &invocation_id,
                ChildCountLimits {
                    logical_requests,
                    tool_calls,
                },
            )
            .await
    }
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
            _ = async {
                match deadline {
                    Some(deadline) => tokio::time::sleep(deadline).await,
                    None => std::future::pending::<()>().await,
                }
            } => {
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
    let result = attach_owner_failure(
        factory,
        run_child_inner(spec, factory, events, child_token, Some(&view), mailbox).await,
    );
    monitor.stop().await;
    let started = ledger.child_started(&agent_id, &invocation_id).await;
    let settlement = ledger
        .finish_child(
            session,
            if matches!(
                &result,
                Err(ChildRunError::Settlement(_)
                    | ChildRunError::Cleanup { .. }
                    | ChildRunError::OwnerOperation { .. }
                    | ChildRunError::Agent(AgentError::EffectOwnerFailure(_)))
            ) {
                EffectOutcome::Unknown
            } else if started {
                EffectOutcome::Succeeded
            } else {
                EffectOutcome::NotStarted
            },
        )
        .await;
    if settlement.is_err() {
        return Err(match result {
            Err(primary) => ChildRunError::Cleanup {
                primary: Box::new(primary),
                kind: ChildCleanupFailure::BudgetPersistence,
            },
            Ok(_) => ChildRunError::OwnerFailure,
        });
    }
    invocation_guard.0 = None;
    result
}

/// The factory snapshot is read only after tool settlement (and, for the
/// budgeted path, mandatory ledger settlement). It therefore cannot erase a
/// real provider/user failure that happened earlier in the same child.
fn attach_owner_failure(
    factory: &dyn ChildFactory,
    result: Result<saya_agent::AgentOutput, ChildRunError>,
) -> Result<saya_agent::AgentOutput, ChildRunError> {
    match factory.owner_failure() {
        Some(failure) => Err(ChildRunError::OwnerOperation {
            prior: result.err().filter(owner_prior_is_genuine).map(Box::new),
            failure,
        }),
        None => result,
    }
}

fn owner_prior_is_genuine(error: &ChildRunError) -> bool {
    match error {
        ChildRunError::Cleanup { primary, .. } => owner_prior_is_genuine(primary),
        ChildRunError::Cancelled {
            reason: CancellationReason::OwnerFailure,
        }
        | ChildRunError::Unsettled
        | ChildRunError::Settlement(_)
        | ChildRunError::OwnerFailure
        | ChildRunError::ProgressFailure
        | ChildRunError::Agent(
            AgentError::Cancelled
            | AgentError::Provider(saya_agent::ProviderError::Cancelled)
            | AgentError::EffectOwnerFailure(_),
        ) => false,
        _ => true,
    }
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
    let authority_refusal_context = spec.authority_refusal_context();
    let (request, limits, answering, preflight, actor_correlation) = spec.into_run_parts();
    let control = AgentRunControl::new(
        limits,
        answering,
        approval.as_ref(),
        &sink,
        cancellation.clone(),
    )
    .with_authority_refusal_context(authority_refusal_context)
    // A child's blocks are the project files and preferences it was given, not
    // database context, so the framing says only that they are untrusted data.
    .with_context_preamble(ContextPreamble::Coding);
    let control = match actor_correlation {
        Some(actor) => control.with_actor_correlation(actor),
        None => control,
    };
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
    // The helper's loop has ended, whatever its outcome: tell the executor so
    // a decorator observing the turn's end (the job's `Stop` hooks) runs at
    // this point.
    tools.turn_finished().await;
    if let Err(cleanup) = tools.settle_invocation().await {
        return Err(match result {
            Err(primary) => ChildRunError::Cleanup {
                primary: Box::new(ChildRunError::Agent(primary)),
                kind: ChildCleanupFailure::Resources,
            },
            Ok(_) => ChildRunError::Settlement(cleanup),
        });
    }
    let reason = cancellation.reason();
    if sink.owner_failure.load(Ordering::Acquire) {
        return Err(match result {
            Err(
                AgentError::Cancelled | AgentError::Provider(saya_agent::ProviderError::Cancelled),
            )
            | Ok(_) => ChildRunError::ProgressFailure,
            Err(primary) => ChildRunError::Cleanup {
                primary: Box::new(ChildRunError::Agent(primary)),
                kind: ChildCleanupFailure::Progress,
            },
        });
    }
    if reason == Some(CancellationReason::OwnerFailure)
        && matches!(
            &result,
            Err(AgentError::Cancelled | AgentError::Provider(saya_agent::ProviderError::Cancelled))
        )
    {
        // Cancellation provenance remains distinct from a failed durable owner.
        // The outer progress owner knows whether its delivery caused this stop.
        return Err(ChildRunError::Cancelled {
            reason: CancellationReason::OwnerFailure,
        });
    }
    if result.is_ok() && reason == Some(CancellationReason::User) {
        return Err(ChildRunError::Cancelled {
            reason: CancellationReason::User,
        });
    }
    if matches!(&result, Err(AgentError::Cancelled))
        && let Some(reason) = reason
    {
        return Err(ChildRunError::Cancelled { reason });
    }
    result.map_err(ChildRunError::Agent)
}

#[cfg(test)]
mod owner_operation_tests {
    use super::*;
    use saya_agent::BudgetDimension;

    #[test]
    fn owner_snapshot_only_keeps_independent_prior_causes() {
        assert!(!owner_prior_is_genuine(&ChildRunError::Unsettled));
        assert!(!owner_prior_is_genuine(&ChildRunError::Cancelled {
            reason: CancellationReason::OwnerFailure,
        }));
        assert!(!owner_prior_is_genuine(&ChildRunError::Agent(
            AgentError::Cancelled
        )));
        assert!(owner_prior_is_genuine(&ChildRunError::Cancelled {
            reason: CancellationReason::User,
        }));
        assert!(owner_prior_is_genuine(&ChildRunError::Cancelled {
            reason: CancellationReason::Budget(BudgetDimension::ElapsedTime),
        }));
        assert!(owner_prior_is_genuine(&ChildRunError::Agent(
            AgentError::Provider(saya_agent::ProviderError::InvalidResponse,)
        )));
    }
}
