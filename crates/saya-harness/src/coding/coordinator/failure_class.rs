//! Bounded classification of a failed child's cause for the joined result.
use super::{api::ChildRunError, schedule_types::ScheduleChildError};
use saya_agent::{AgentError, AgentOutput, ProviderError};
use saya_types::{CodingFailureClass, CodingResultOutcome};

/// The class of a child error; every error maps to one closed class, never to its text.
pub(super) fn classify(error: &ScheduleChildError) -> Option<CodingFailureClass> {
    use CodingFailureClass as Class;
    Some(match error {
        ScheduleChildError::Run(run) => match run {
            ChildRunError::Agent(agent) => agent_class(agent),
            ChildRunError::BudgetDenied => Class::AdmissionDenied,
            ChildRunError::Factory(_) | ChildRunError::ChannelIdentityMismatch => {
                Class::SetupFailed
            }
            ChildRunError::Cancelled { .. } => Class::Cancelled,
            _ => Class::Internal,
        },
        _ => Class::Internal,
    })
}

fn agent_class(error: &AgentError) -> CodingFailureClass {
    use CodingFailureClass as Class;
    match error {
        AgentError::Provider(ProviderError::OutputTruncated { .. }) => Class::OutputLimit,
        AgentError::Provider(_) => Class::ProviderError,
        AgentError::Limit(_) => Class::LimitReached,
        AgentError::InvalidToolCall | AgentError::InvalidHistory => Class::InvalidToolCall,
        AgentError::EffectAdmissionDenied => Class::AdmissionDenied,
        AgentError::UnsupportedPhysicalCoverage => Class::Unsupported,
        AgentError::Cancelled => Class::Cancelled,
        _ => Class::Internal,
    }
}

/// The class to persist with a child's result: only a `Failed` outcome carries one. Uncertain,
/// completed and user-cancelled results keep it unset.
pub(super) fn classify_result(
    result: &Result<AgentOutput, ScheduleChildError>,
    outcome: CodingResultOutcome,
) -> Option<CodingFailureClass> {
    if outcome != CodingResultOutcome::Failed {
        return None;
    }
    match result {
        Ok(output) if output.truncated => Some(CodingFailureClass::OutputLimit),
        Ok(_) => None,
        Err(error) => classify(error),
    }
}

#[cfg(test)]
#[path = "failure_class_tests.rs"]
mod tests;
