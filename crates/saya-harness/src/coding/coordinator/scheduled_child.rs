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
        finish_progress(result, progress_failed)
    };
    super::cancel::join_with_cancellation(joined, root, cancellation.clone()).await
}

// Reconcile owner results only after both the loop and progress drain finish.
fn finish_progress(
    result: Result<saya_agent::AgentOutput, ChildRunError>,
    progress_failed: bool,
) -> Result<saya_agent::AgentOutput, ChildRunError> {
    if progress_failed {
        Err(match result {
            Err(
                ChildRunError::ProgressFailure
                | ChildRunError::Agent(
                    saya_agent::AgentError::Cancelled
                    | saya_agent::AgentError::Provider(saya_agent::ProviderError::Cancelled),
                )
                | ChildRunError::Cancelled {
                    reason: CancellationReason::OwnerFailure,
                },
            ) => ChildRunError::ProgressFailure,
            Err(primary) => ChildRunError::Cleanup {
                primary: Box::new(primary),
                kind: super::api::ChildCleanupFailure::Progress,
            },
            Ok(_) => ChildRunError::ProgressFailure,
        })
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use saya_agent::{AgentError, ProviderError};
    #[test]
    fn progress_failure_preserves_established_primary_and_cleanup_causes() {
        let primary = ChildRunError::Agent(AgentError::Provider(ProviderError::Request(
            "PRIVATE_PROVIDER_SENTINEL".into(),
        )));
        let result = finish_progress(Err(primary), true).unwrap_err();
        assert!(
            matches!(&result,ChildRunError::Cleanup {primary,kind:super::super::api::ChildCleanupFailure::Progress} if matches!(primary.as_ref(),ChildRunError::Agent(AgentError::Provider(ProviderError::Request(_))))),
            "progress erased primary: {result:?}"
        );
        let result = super::super::ScheduleChildError::Run(result);
        assert_eq!(
            super::super::failure_class::classify(&result),
            Some(saya_types::CodingFailureClass::ProviderError)
        );
        let envelope = super::super::results::envelope(
            saya_types::JobId::parse("job-a").unwrap(),
            saya_types::AgentId::parse("child-a").unwrap(),
            saya_types::InvocationId::parse("invocation-a").unwrap(),
            &Err(result),
        )
        .unwrap();
        assert_eq!(
            envelope.outcome(),
            saya_types::CodingResultOutcome::Uncertain
        );
        assert!(
            !String::from_utf8(envelope.to_json_bytes().unwrap())
                .unwrap()
                .contains("PRIVATE_PROVIDER_SENTINEL")
        );
        let cancelled = finish_progress(
            Err(ChildRunError::Agent(AgentError::Provider(
                ProviderError::Cancelled,
            ))),
            true,
        )
        .unwrap_err();
        assert!(
            matches!(cancelled, ChildRunError::ProgressFailure),
            "progress cancellation alias was misclassified: {cancelled:?}"
        );
        let mandatory = finish_progress(Err(ChildRunError::OwnerFailure), true).unwrap_err();
        assert_eq!(
            super::super::failure_class::classify(&super::super::ScheduleChildError::Run(
                mandatory
            )),
            Some(saya_types::CodingFailureClass::MandatoryPersistence)
        );
        let primary = ChildRunError::Cleanup {
            primary: Box::new(ChildRunError::Agent(AgentError::Provider(
                ProviderError::InvalidResponse,
            ))),
            kind: super::super::api::ChildCleanupFailure::Resources,
        };
        let result = finish_progress(Err(primary), true).unwrap_err();
        assert!(
            matches!(&result,ChildRunError::Cleanup {primary,kind:super::super::api::ChildCleanupFailure::Progress} if matches!(primary.as_ref(),ChildRunError::Cleanup {kind:super::super::api::ChildCleanupFailure::Resources,..})),
            "progress erased cleanup uncertainty: {result:?}"
        );
        let envelope = super::super::results::envelope(
            saya_types::JobId::parse("job-a").unwrap(),
            saya_types::AgentId::parse("child-a").unwrap(),
            saya_types::InvocationId::parse("invocation-a").unwrap(),
            &Err(super::super::ScheduleChildError::Run(result)),
        )
        .unwrap();
        assert_eq!(
            envelope.outcome(),
            saya_types::CodingResultOutcome::Uncertain
        );
    }
}
