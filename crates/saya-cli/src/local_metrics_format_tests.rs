use crate::local_metrics::LocalMetrics;
use saya_agent::{
    AgentAttemptOutcome, AgentMetric, AgentMetricCount, AgentMetricOutcome, AgentMetricPhase,
    AgentMetricUseful,
};

#[path = "local_metrics_format_tests/saturation.rs"]
mod saturation;
#[path = "local_metrics_format_tests/slot_order.rs"]
mod slot_order;
#[path = "local_metrics_format_tests/unrecorded_phase.rs"]
mod unrecorded_phase;
#[path = "local_metrics_format_tests/vocabulary.rs"]
mod vocabulary;

/// Every indexed phase, in `phase_index` slot order.
const ALL_PHASES: [AgentMetricPhase; 14] = [
    AgentMetricPhase::ProviderEstablishment,
    AgentMetricPhase::ProviderReadAwait,
    AgentMetricPhase::FirstProviderEvent,
    AgentMetricPhase::LocalAttempt,
    AgentMetricPhase::RetryBackoff,
    AgentMetricPhase::DatabaseConnect,
    AgentMetricPhase::DatabaseWait,
    AgentMetricPhase::DatabaseExecute,
    AgentMetricPhase::SchemaDiscovery,
    AgentMetricPhase::ApprovalWait,
    AgentMetricPhase::Compaction,
    AgentMetricPhase::FirstPaint,
    AgentMetricPhase::QuestionFirstUseful,
    AgentMetricPhase::CancellationToLocalSettlement,
];

/// Every indexed count, in `count_index` slot order.
const ALL_COUNTS: [AgentMetricCount; 5] = [
    AgentMetricCount::RetryAttemptStarted,
    AgentMetricCount::OutputTruncated,
    AgentMetricCount::AdmittedContinuation,
    AgentMetricCount::CompactionStarted,
    AgentMetricCount::ToolResultTruncated,
];

/// The snake_case label the formatter must print for each indexed slot: the
/// test's own table, aligned with the index functions by construction of the
/// arrays above.
const PHASE_WORDS: [&str; 14] = [
    "provider_establishment",
    "provider_read_await",
    "first_provider_event",
    "local_attempt",
    "retry_backoff",
    "database_connect",
    "database_wait",
    "database_execute",
    "schema_discovery",
    "approval_wait",
    "compaction",
    "first_paint",
    "question_first_useful",
    "cancellation_to_local_settlement",
];

const COUNT_WORDS: [&str; 5] = [
    "retry_attempt_started",
    "output_truncated",
    "admitted_continuation",
    "compaction_started",
    "tool_result_truncated",
];

const COVERAGE_WORDS: [&str; 3] = ["succeeded", "failed", "cancelled"];

/// Feeds one sample of every known phase, every count, all three coverage
/// outcomes, and both useful kinds into one collector and returns its
/// snapshot.
fn full_snapshot() -> crate::local_metrics::LocalPhaseSnapshot {
    let metrics = LocalMetrics::new();
    for phase in ALL_PHASES {
        metrics.record(&AgentMetric::Phase {
            phase,
            elapsed_ns: 1234,
            outcome: AgentMetricOutcome::Succeeded,
        });
    }
    for count in ALL_COUNTS {
        metrics.record(&AgentMetric::Count(count));
    }
    metrics.record(&AgentMetric::Useful(AgentMetricUseful::NonWhitespaceText));
    metrics.record(&AgentMetric::Useful(AgentMetricUseful::SuccessfulTool));
    metrics.record(&AgentMetric::UsageCoverage {
        attempt_outcome: AgentAttemptOutcome::Succeeded,
        reported: true,
    });
    metrics.record(&AgentMetric::UsageCoverage {
        attempt_outcome: AgentAttemptOutcome::Failed,
        reported: false,
    });
    metrics.snapshot()
}

fn line_starting_with<'a>(output: &'a str, prefix: &str) -> &'a str {
    output
        .lines()
        .find(|line| line.starts_with(prefix))
        .unwrap_or_else(|| panic!("no line starts with {prefix:?}: {output}"))
}
