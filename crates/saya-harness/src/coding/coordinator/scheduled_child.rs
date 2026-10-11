//! One owned loop and its concurrent bounded progress drain.
use super::{
    api::{ChildRunError, run_child_controlled},
    events::{ChildEvent, child_event_channel},
    schedule_types::ScheduledChild,
};
use crate::coding::budget::JobBudgetLedger;
use saya_agent::{CancellationReason, CancellationToken};
use saya_types::{CodingEventKind, CodingEventV1, JobState};
use std::num::NonZeroU64;
use tokio::sync::mpsc;

pub(super) async fn run(
    node: ScheduledChild,
    ledger: JobBudgetLedger,
    root: CancellationToken,
    progress: mpsc::Sender<ChildEvent>,
) -> Result<saya_agent::AgentOutput, ChildRunError> {
    let authority = node.spec.authority();
    let job = authority.job_id().clone();
    let agent = authority.agent_id().clone();
    let invocation = authority.invocation_id().clone();
    ledger
        .sequencer()
        .append_event(|sequence| {
            CodingEventV1::new(
                NonZeroU64::new(sequence).expect("sequencer allocated nonzero"),
                job,
                Some(agent),
                Some(invocation),
                CodingEventKind::StateChanged {
                    state: JobState::Running,
                },
            )
            .expect("validated child identities and state")
        })
        .await
        .map_err(|_| ChildRunError::OwnerFailure)?;
    let cancellation = node.control.cancellation.clone();
    let (sender, mut receiver) = child_event_channel(&node.spec);
    let loop_future = run_child_controlled(
        node.spec,
        node.factory.as_ref(),
        sender,
        cancellation.clone(),
        &ledger,
        Some(node.mailbox),
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
    let joined = async {
        let (result, progress_failed) = tokio::join!(loop_future, drain);
        if progress_failed {
            Err(ChildRunError::OwnerFailure)
        } else {
            result
        }
    };
    super::cancel::join_with_cancellation(joined, root, cancellation.clone()).await
}
