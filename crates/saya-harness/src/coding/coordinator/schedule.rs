//! One-shot bounded child dispatch with durable joined results. The session
//! that admits children while it runs is in `session.rs`; this is that session
//! opened on one batch and closed immediately.
use super::super::budget::JobBudgetLedger;
use super::{
    api::ChildRunError,
    events::ChildEvent,
    schedule_types::{
        ScheduleChildError, ScheduleError, ScheduleLimits, ScheduleReport, ScheduleStop,
        ScheduledChild,
    },
    session::ScheduleSession,
};
use saya_agent::{AgentOutput, CancellationReason, CancellationToken};
use tokio::sync::mpsc;

/// Runs one trusted batch and joins its started work before normal return.
/// All native job children route here and provider keys are bound to factories;
/// separate direct child calls bypass these in-memory path/provider leases.
/// Capacity leaves coordinator headroom, not a physically reserved model permit.
/// Root progress must be consumed promptly: full/closed queues cancel the batch.
/// Dropping this future aborts local tasks and fails the ledger closed; it does
/// not claim durable settlement, remote termination, or crash reconciliation.
pub async fn run_schedule(
    nodes: Vec<ScheduledChild>,
    limits: ScheduleLimits,
    ledger: &JobBudgetLedger,
    caller: CancellationToken,
    progress: mpsc::Sender<ChildEvent>,
) -> Result<ScheduleReport, ScheduleError> {
    let (session, run, completions) =
        ScheduleSession::open(nodes, limits, ledger, caller, progress).await?;
    // The batch is the whole session: dropping the only submit side closes it,
    // and every child of the batch is already in it.
    drop(session);
    drop(completions);
    let mut report = run.run().await?;
    // A one-shot batch keeps the batch-level `ChildFailed` stop: the session
    // records each failure against its own child, and here nothing is left to
    // wait for. An individually cancelled child is not a batch failure.
    if report.stop.is_none()
        && report
            .outcomes
            .iter()
            .any(|outcome| batch_failure(&outcome.result))
    {
        report.stop = Some(ScheduleStop::ChildFailed);
    }
    Ok(report)
}

fn batch_failure(result: &Result<AgentOutput, ScheduleChildError>) -> bool {
    result.is_err()
        && !matches!(
            result,
            Err(ScheduleChildError::Run(ChildRunError::Cancelled {
                reason: CancellationReason::User,
            }))
        )
}
