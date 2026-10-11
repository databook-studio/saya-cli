//! One owned loop and its concurrent bounded progress drain.
use super::super::budget::JobBudgetLedger;
use super::{
    api::{ChildRunError, run_child_budgeted},
    events::{ChildEvent, child_event_channel},
    schedule_types::ScheduledChild,
};
use saya_agent::{CancellationReason, CancellationToken};
use tokio::sync::mpsc;

pub(super) async fn run(
    node: ScheduledChild,
    ledger: JobBudgetLedger,
    cancellation: CancellationToken,
    progress: mpsc::Sender<ChildEvent>,
) -> Result<saya_agent::AgentOutput, ChildRunError> {
    let (sender, mut receiver) = child_event_channel(&node.spec);
    let loop_future = run_child_budgeted(
        node.spec,
        node.factory.as_ref(),
        sender,
        cancellation.clone(),
        &ledger,
    );
    let drain = async {
        let mut failed = false;
        while let Some(event) = receiver.recv().await {
            if !failed && progress.try_send(event).is_err() {
                failed = true;
                cancellation.cancel_with_reason(CancellationReason::OwnerFailure);
            }
        }
        failed
    };
    let (result, progress_failed) = tokio::join!(loop_future, drain);
    if progress_failed {
        Err(ChildRunError::OwnerFailure)
    } else {
        result
    }
}
