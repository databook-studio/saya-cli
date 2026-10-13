use super::{admit::NodePlan, api::ChildRunError, schedule_types::ScheduleChildError};
use crate::coding::budget::{JobBudgetLedger, SequencerError};
use saya_agent::{AgentOutput, CancellationReason};
use saya_types::{
    AgentId, CodingResultOutcome, InvocationId, JobId, JoinedResultV1, MAX_JOINED_ANSWER_BYTES,
};
use std::num::NonZeroU64;

#[derive(Debug, thiserror::Error)]
pub enum CompletionError {
    #[error("joined result failed its bounded contract")]
    Envelope(#[from] saya_types::JoinedResultError),
    #[error("joined result could not be made durable")]
    Store(#[from] SequencerError),
}

pub(super) async fn persist(
    ledger: &JobBudgetLedger,
    plan: &NodePlan,
    result: &Result<AgentOutput, ScheduleChildError>,
) -> Result<NonZeroU64, CompletionError> {
    let envelope = envelope(
        ledger.job_id().await,
        plan.agent.clone(),
        plan.invocation.clone(),
        result,
    )?;
    Ok(ledger
        .sequencer()
        .append_completion(&envelope)
        .await?
        .sequence)
}

/// The bounded joined result for one finished child: its outcome, a redacted answer, and, for a
/// failed child, the closed failure class. Pure so the mapping is testable without a store.
pub(super) fn envelope(
    job: JobId,
    agent: AgentId,
    invocation: InvocationId,
    result: &Result<AgentOutput, ScheduleChildError>,
) -> Result<JoinedResultV1, saya_types::JoinedResultError> {
    let outcome = match result {
        Ok(output) if output.truncated => CodingResultOutcome::Failed,
        Ok(_) => CodingResultOutcome::Completed,
        Err(
            ScheduleChildError::Join(_)
            | ScheduleChildError::Run(
                ChildRunError::Unsettled
                | ChildRunError::Settlement(_)
                | ChildRunError::Agent(saya_agent::AgentError::EffectOwnerFailure(_)),
            ),
        ) => CodingResultOutcome::Uncertain,
        Err(ScheduleChildError::Run(ChildRunError::Cancelled {
            reason: CancellationReason::User,
        })) => CodingResultOutcome::Cancelled,
        Err(_) => CodingResultOutcome::Failed,
    };
    // Never persist raw provider/tool errors or trace payloads. Oversized answers
    // are omitted, not sliced through a credential; redaction operates on a
    // bounded complete string. Callers see explicit omission in the envelope.
    let (answer, truncated) = match result {
        Ok(output) if output.answer.len() <= MAX_JOINED_ANSWER_BYTES => {
            let answer = saya_types::redact(&output.answer);
            if answer.len() <= MAX_JOINED_ANSWER_BYTES {
                (answer, false)
            } else {
                (String::new(), true)
            }
        }
        Ok(_) => (String::new(), true),
        Err(_) => (String::new(), false),
    };
    let envelope = JoinedResultV1::new(job, agent, invocation, outcome, answer, truncated)?;
    Ok(
        match super::failure_class::classify_result(result, outcome) {
            Some(class) => envelope.with_failure_class(class),
            None => envelope,
        },
    )
}

#[cfg(test)]
#[path = "results_tests.rs"]
mod tests;
