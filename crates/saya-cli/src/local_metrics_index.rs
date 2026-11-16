use saya_agent::{AgentAttemptOutcome, AgentMetricCount, AgentMetricOutcome, AgentMetricPhase};

pub(crate) fn phase_index(phase: AgentMetricPhase) -> Option<usize> {
    match phase {
        AgentMetricPhase::ProviderEstablishment => Some(0),
        AgentMetricPhase::ProviderReadAwait => Some(1),
        AgentMetricPhase::FirstProviderEvent => Some(2),
        AgentMetricPhase::LocalAttempt => Some(3),
        AgentMetricPhase::RetryBackoff => Some(4),
        AgentMetricPhase::DatabaseConnect => Some(5),
        AgentMetricPhase::DatabaseWait => Some(6),
        AgentMetricPhase::DatabaseExecute => Some(7),
        AgentMetricPhase::SchemaDiscovery => Some(8),
        AgentMetricPhase::ApprovalWait => Some(9),
        AgentMetricPhase::Compaction => Some(10),
        AgentMetricPhase::FirstPaint => Some(11),
        AgentMetricPhase::QuestionFirstUseful => Some(12),
        AgentMetricPhase::CancellationToLocalSettlement => Some(13),
        _ => None,
    }
}

pub(crate) fn outcome_index(outcome: AgentMetricOutcome) -> Option<usize> {
    match outcome {
        AgentMetricOutcome::Succeeded => Some(0),
        AgentMetricOutcome::Failed => Some(1),
        AgentMetricOutcome::Cancelled => Some(2),
        AgentMetricOutcome::Unavailable => Some(3),
        AgentMetricOutcome::CachedFallback => Some(4),
        AgentMetricOutcome::Skipped => Some(5),
        _ => None,
    }
}

pub(crate) fn count_index(count: AgentMetricCount) -> Option<usize> {
    match count {
        AgentMetricCount::RetryAttemptStarted => Some(0),
        AgentMetricCount::OutputTruncated => Some(1),
        AgentMetricCount::AdmittedContinuation => Some(2),
        AgentMetricCount::CompactionStarted => Some(3),
        AgentMetricCount::ToolResultTruncated => Some(4),
        _ => None,
    }
}

pub(crate) fn coverage_index(outcome: AgentAttemptOutcome) -> Option<usize> {
    match outcome {
        AgentAttemptOutcome::Succeeded => Some(0),
        AgentAttemptOutcome::Failed => Some(1),
        AgentAttemptOutcome::Cancelled => Some(2),
        _ => None,
    }
}
