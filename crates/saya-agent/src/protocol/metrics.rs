//! Fixed source-owned metrics for local agent execution.

/// A measured agent phase. Values are local monotonic elapsed nanoseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentMetric {
    Phase {
        phase: AgentMetricPhase,
        elapsed_ns: u64,
        outcome: AgentMetricOutcome,
    },
    Count(AgentMetricCount),
    Useful(AgentMetricUseful),
    UsageCoverage {
        attempt_outcome: AgentAttemptOutcome,
        reported: bool,
    },
}

/// Provider or retry source boundary measured by a phase record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentMetricPhase {
    ProviderEstablishment,
    ProviderReadAwait,
    FirstProviderEvent,
    LocalAttempt,
    RetryBackoff,
}

/// Local result of a measured phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentMetricOutcome {
    Succeeded,
    Failed,
    Cancelled,
    Unavailable,
}

/// Fixed agent-owned count observations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentMetricCount {
    RetryAttemptStarted,
    OutputTruncated,
    AdmittedContinuation,
}

/// A first-useful observation based on typed source results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentMetricUseful {
    NonWhitespaceText,
    SuccessfulTool,
}

/// Terminal outcome of one local provider attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentAttemptOutcome {
    Succeeded,
    Failed,
    Cancelled,
}
