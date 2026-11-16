use super::line_starting_with;
use crate::local_metrics::LocalMetrics;
use crate::local_metrics_format::format_snapshot;
use saya_agent::{AgentMetric, AgentMetricOutcome, AgentMetricPhase};

/// (c) Saturated cells print without overflow or panic: u64::MAX nanoseconds
/// divide into a whole-microsecond count and u64::MAX counters print as-is.
#[test]
fn saturated_cells_print_without_overflow() {
    let snapshot = crate::local_metrics::LocalPhaseSnapshot {
        counts: [u64::MAX; 5],
        ..Default::default()
    };
    let metrics = LocalMetrics::new();
    metrics.record(&AgentMetric::Phase {
        phase: AgentMetricPhase::ProviderEstablishment,
        elapsed_ns: u64::MAX,
        outcome: AgentMetricOutcome::Succeeded,
    });
    metrics.record(&AgentMetric::Phase {
        phase: AgentMetricPhase::ProviderEstablishment,
        elapsed_ns: u64::MAX,
        outcome: AgentMetricOutcome::Succeeded,
    });
    let mut saturated = metrics.snapshot();
    saturated.counts = snapshot.counts;
    let output = format_snapshot(&saturated);
    assert!(
        output.contains(&format!("total_us={}", u64::MAX / 1_000)),
        "the saturated duration divides into whole microseconds: {output}"
    );
    assert!(
        line_starting_with(&output, "count retry_attempt_started").contains(&u64::MAX.to_string()),
        "a saturated counter prints its full value: {output}"
    );
}
