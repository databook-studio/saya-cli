//! Bounded classification of a failed child's cause for the joined result.
use super::{api::ChildRunError, schedule_types::ScheduleChildError};
use saya_agent::{AgentError, AgentOutput, CancellationReason, ProviderError};
use saya_types::{CodingFailureClass, CodingResultOutcome};

/// The class of a child error; every error maps to one closed class, never to its text.
pub fn classify(error: &ScheduleChildError) -> Option<CodingFailureClass> {
    use CodingFailureClass as Class;
    Some(match error {
        ScheduleChildError::Run(run) => child_class(run),
        ScheduleChildError::Incomplete => Class::OutputLimit,
        ScheduleChildError::BudgetStopped => Class::BudgetStopped,
        _ => Class::Internal,
    })
}
fn child_class(run: &ChildRunError) -> CodingFailureClass {
    use CodingFailureClass as Class;
    match run {
        ChildRunError::Agent(agent) => agent_class(agent),
        ChildRunError::Cleanup { primary, .. } => child_class(primary),
        ChildRunError::OwnerOperation { prior, failure } => prior
            .as_deref()
            .map_or_else(|| failure.primary(), child_class),
        ChildRunError::BudgetDenied => Class::AdmissionDenied,
        ChildRunError::OwnerFailure => Class::MandatoryPersistence,
        ChildRunError::ProgressFailure => Class::ProgressDelivery,
        ChildRunError::Unsettled | ChildRunError::Settlement(_) => Class::EffectsUncertain,
        ChildRunError::Factory(_) | ChildRunError::ChannelIdentityMismatch => Class::SetupFailed,
        // Preserve a budget stop independently of ordinary cancellation.
        ChildRunError::Cancelled {
            reason: CancellationReason::Budget(_),
        } => Class::BudgetStopped,
        ChildRunError::Cancelled { .. } => Class::Cancelled,
    }
}

pub fn agent_class(error: &AgentError) -> CodingFailureClass {
    use CodingFailureClass as Class;
    match error {
        AgentError::Provider(ProviderError::OutputTruncated { .. }) => Class::OutputLimit,
        AgentError::Provider(ProviderError::Cancelled) => Class::Cancelled,
        // A cut that exhausted its continuations stopped the run mid-answer
        // exactly as a stream that ended with nothing did, so it keeps the
        // same class: the operator's guidance and the recorded category are
        // unchanged, and no new wire value is introduced.
        AgentError::Provider(ProviderError::StreamEnded | ProviderError::StreamCut { .. }) => {
            Class::ProviderStreamEnded
        }
        AgentError::Provider(_) => Class::ProviderError,
        AgentError::Limit(_) => Class::LimitReached,
        AgentError::InvalidToolCall | AgentError::InvalidHistory => Class::InvalidToolCall,
        AgentError::EffectAdmissionDenied => Class::AdmissionDenied,
        AgentError::UnsupportedPhysicalCoverage => Class::Unsupported,
        AgentError::Cancelled => Class::Cancelled,
        AgentError::EffectOwnerFailure(_) => Class::MandatoryPersistence,
        AgentError::Progress(stop) => progress_class(stop.kind),
        // A future agent-layer error still failed; the closed internal class
        // keeps it out of the success path without inventing a new wire value.
        _ => Class::Internal,
    }
}

/// A hard progress guard's class. The clean stops return `Ok` and are
/// reported by the caller's progress handle instead.
fn progress_class(kind: saya_agent::ProgressStopKind) -> CodingFailureClass {
    use CodingFailureClass as Class;
    match kind {
        saya_agent::ProgressStopKind::NoProgress => Class::NoProgress,
        saya_agent::ProgressStopKind::Repeating => Class::Repeating,
        saya_agent::ProgressStopKind::WallClock => Class::WallClock,
        saya_agent::ProgressStopKind::Cost => Class::Cost,
    }
}

/// The class to persist with a child's result. A `Failed` or `Uncertain` outcome carries the
/// class of its cause — for an uncertain child, the latched failure that left its effect
/// unsettled. Completed and user-cancelled results keep it unset.
pub(super) fn classify_result(
    result: &Result<AgentOutput, ScheduleChildError>,
    outcome: CodingResultOutcome,
) -> Option<CodingFailureClass> {
    match outcome {
        CodingResultOutcome::Failed => match result {
            Ok(output) if output.truncated => Some(if budget_stopped(output) {
                CodingFailureClass::BudgetStopped
            } else {
                CodingFailureClass::OutputLimit
            }),
            Ok(_) => None,
            Err(error) => classify(error),
        },
        CodingResultOutcome::Uncertain => match result {
            Err(error) => classify(error),
            Ok(_) => None,
        },
        CodingResultOutcome::Completed | CodingResultOutcome::Cancelled => None,
        // `CodingResultOutcome` is non-exhaustive; a future outcome carries no class until it is
        // classified here deliberately.
        _ => None,
    }
}

pub(super) fn budget_stopped(output: &AgentOutput) -> bool {
    output
        .events
        .iter()
        .any(|event| matches!(event, saya_agent::AgentEvent::AnsweringBudgetStopped { .. }))
}

#[cfg(test)]
#[path = "failure_class_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "failure_class_persisted_tests.rs"]
mod persisted_tests;
