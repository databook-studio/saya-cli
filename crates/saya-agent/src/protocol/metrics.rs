//! Fixed source-owned metrics for local agent execution.

/// A fixed local metric record. Durations are monotonic elapsed nanoseconds.
#[non_exhaustive]
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

/// A fixed source boundary measured by a phase record.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentMetricPhase {
    ProviderEstablishment,
    ProviderReadAwait,
    FirstProviderEvent,
    LocalAttempt,
    RetryBackoff,
    /// One actual eager connector connection await, through its local return.
    DatabaseConnect,
    /// Registry lease wait through acquisition or cancellation; uncontended may be zero.
    DatabaseWait,
    /// Connector execution after lease acquisition through local source settlement.
    DatabaseExecute,
    /// Live schema discovery work; cached fallback is a distinct outcome.
    SchemaDiscovery,
    /// A real user or channel approval wait, excluding automatic policy decisions.
    ApprovalWait,
    /// Compaction work through validated result construction/application, excluding UI polling.
    Compaction,
    /// Session entry through the first successful terminal draw.
    FirstPaint,
    /// Wall elapsed from question start to the first source proxy, not active budget or correctness.
    QuestionFirstUseful,
    /// Local time from accepted cancellation to settlement; unavailable without its request time.
    CancellationToLocalSettlement,
}

/// Local result of a measured phase.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentMetricOutcome {
    Succeeded,
    Failed,
    Cancelled,
    Unavailable,
    /// A cached schema was used; this does not represent a successful live fetch.
    CachedFallback,
    /// The decision skipped work, rather than measuring a successful zero-duration operation.
    Skipped,
}

/// Fixed agent-owned count observations.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentMetricCount {
    RetryAttemptStarted,
    OutputTruncated,
    AdmittedContinuation,
    /// A compaction operation actually began; a declined guard is skipped instead.
    CompactionStarted,
    /// A result was actually cut by its bounded result allocation.
    ToolResultTruncated,
}

/// A first-useful observation based on typed source results.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentMetricUseful {
    NonWhitespaceText,
    SuccessfulTool,
}

/// Terminal outcome of one local provider attempt.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentAttemptOutcome {
    Succeeded,
    Failed,
    Cancelled,
}
