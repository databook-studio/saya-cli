use super::line_starting_with;
use crate::local_metrics::LocalMetrics;
use crate::local_metrics_format::format_snapshot;
use saya_agent::{AgentMetric, AgentMetricOutcome, AgentMetricPhase};

/// (b) A never-recorded phase prints the literal `unavailable` with no digit,
/// while a recorded zero-duration Succeeded sample prints a numeric 0: known
/// zero is not absence.
#[test]
fn an_unrecorded_phase_prints_unavailable_and_a_known_zero_prints_zero() {
    let metrics = LocalMetrics::new();
    metrics.record(&AgentMetric::Phase {
        phase: AgentMetricPhase::LocalAttempt,
        elapsed_ns: 0,
        outcome: AgentMetricOutcome::Succeeded,
    });
    let output = format_snapshot(&metrics.snapshot());
    let absent = line_starting_with(&output, "phase first_paint");
    assert!(
        absent.contains("unavailable"),
        "a never-recorded phase is unavailable: {absent}"
    );
    assert!(
        !absent.chars().any(|c| c.is_ascii_digit()),
        "absence is not a zero: {absent}"
    );
    let known = line_starting_with(&output, "phase local_attempt");
    assert!(
        known.contains("samples=1") && known.contains("total_us=0"),
        "a recorded zero-duration sample is a known zero: {known}"
    );
    assert!(
        !line_starting_with(&output, "phase provider_establishment")
            .chars()
            .any(|c| c.is_ascii_digit()),
        "another never-recorded phase is unavailable too: {output}"
    );
}
