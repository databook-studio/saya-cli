use super::*;
use saya_agent::{AnsweringScope, run_agent_with_control};

async fn run_in_scope(
    provider: &dyn ChatProvider,
    executor: &dyn ToolExecutor,
    sink: &RecordingSink,
    scope: &AnsweringScope,
    answering: AnsweringLimits,
) -> Result<saya_agent::AgentOutput, AgentError> {
    run_in_scope_with_token(
        provider,
        executor,
        sink,
        scope,
        answering,
        scope.cancellation_token(),
    )
    .await
}

async fn run_in_scope_with_token(
    provider: &dyn ChatProvider,
    executor: &dyn ToolExecutor,
    sink: &RecordingSink,
    scope: &AnsweringScope,
    answering: AnsweringLimits,
    control_token: CancellationToken,
) -> Result<saya_agent::AgentOutput, AgentError> {
    run_agent_with_control(
        provider,
        executor,
        request(),
        definitions(),
        AgentRunControl::new(
            AgentLimits::default(),
            answering,
            &AllowReadOnlyApproval,
            sink,
            control_token,
        )
        .with_answering_scope(scope.clone()),
    )
    .await
}

fn request_count(observations: &[AnsweringObservation]) -> usize {
    observations
        .iter()
        .filter(|observation| matches!(observation, AnsweringObservation::RequestAdmitted))
        .count()
}

fn batch_counts(observations: &[AnsweringObservation]) -> Vec<u64> {
    observations
        .iter()
        .filter_map(|observation| match observation {
            AnsweringObservation::ToolBatchObserved { requested } => Some(*requested),
            _ => None,
        })
        .collect()
}

#[path = "shared_scope/shared_cancellation.rs"]
mod shared_cancellation;
#[path = "shared_scope/shared_limits.rs"]
mod shared_limits;
#[path = "shared_scope/shared_receipts.rs"]
mod shared_receipts;
