use crate::local_metrics::{
    LocalMetrics, LocalPhaseSnapshot, count_index, coverage_index, enabled, outcome_index,
    phase_index, set_enabled,
};
use saya_agent::{
    AgentAttemptOutcome, AgentMetric, AgentMetricCount, AgentMetricOutcome, AgentMetricPhase,
    AgentMetricUseful,
};

fn cell(snapshot: &LocalPhaseSnapshot, phase: AgentMetricPhase) -> crate::local_metrics::PhaseCell {
    snapshot.phases[phase_index(phase).expect("known phase indexed")]
}

#[test]
fn phase_total_saturates_without_wrapping() {
    let metrics = LocalMetrics::new();
    let sample = AgentMetric::Phase {
        phase: AgentMetricPhase::ProviderEstablishment,
        elapsed_ns: u64::MAX,
        outcome: AgentMetricOutcome::Succeeded,
    };
    metrics.record(&sample);
    metrics.record(&sample);
    let snapshot = metrics.snapshot();
    let establishment = cell(&snapshot, AgentMetricPhase::ProviderEstablishment);
    assert_eq!(establishment.samples, 2);
    assert_eq!(establishment.total_ns, u64::MAX, "total saturates, no wrap");
    assert_eq!(establishment.max_ns, u64::MAX);
    assert_eq!(snapshot.unrecognized, 0);
}

#[test]
fn known_zero_sample_is_seen_and_an_unrecorded_phase_is_unavailable() {
    let metrics = LocalMetrics::new();
    metrics.record(&AgentMetric::Phase {
        phase: AgentMetricPhase::LocalAttempt,
        elapsed_ns: 0,
        outcome: AgentMetricOutcome::Succeeded,
    });
    let snapshot = metrics.snapshot();
    let attempt = cell(&snapshot, AgentMetricPhase::LocalAttempt);
    assert_eq!(attempt.samples, 1, "a recorded sample is seen");
    assert_eq!(attempt.total_ns, 0, "known zero, not absence");
    let paint = cell(&snapshot, AgentMetricPhase::FirstPaint);
    assert_eq!(paint.samples, 0, "a phase never recorded stays unavailable");
    assert_eq!(
        snapshot,
        metrics.snapshot(),
        "snapshot is a fixed Copy value"
    );
}

#[test]
fn only_the_first_useful_record_is_kept() {
    let metrics = LocalMetrics::new();
    metrics.record(&AgentMetric::Useful(AgentMetricUseful::NonWhitespaceText));
    metrics.record(&AgentMetric::Useful(AgentMetricUseful::SuccessfulTool));
    let first = metrics
        .snapshot()
        .first_useful
        .expect("first useful is set");
    assert_eq!(first.0, AgentMetricUseful::NonWhitespaceText);
    let other = LocalMetrics::new();
    other.record(&AgentMetric::Useful(AgentMetricUseful::SuccessfulTool));
    assert_eq!(
        other.snapshot().first_useful.map(|(kind, _)| kind),
        Some(AgentMetricUseful::SuccessfulTool),
        "either kind can be first"
    );
}

#[test]
fn usage_coverage_counts_reported_and_missing_per_outcome() {
    let metrics = LocalMetrics::new();
    for _ in 0..2 {
        metrics.record(&AgentMetric::UsageCoverage {
            attempt_outcome: AgentAttemptOutcome::Succeeded,
            reported: true,
        });
    }
    metrics.record(&AgentMetric::UsageCoverage {
        attempt_outcome: AgentAttemptOutcome::Succeeded,
        reported: false,
    });
    metrics.record(&AgentMetric::UsageCoverage {
        attempt_outcome: AgentAttemptOutcome::Failed,
        reported: false,
    });
    let snapshot = metrics.snapshot();
    let succeeded = &snapshot.coverage[coverage_index(AgentAttemptOutcome::Succeeded).unwrap()];
    assert!(succeeded.seen);
    assert_eq!(succeeded.reported, 2);
    assert_eq!(succeeded.missing, 1);
    let failed = &snapshot.coverage[coverage_index(AgentAttemptOutcome::Failed).unwrap()];
    assert!(failed.seen, "seen with a known zero reported, not absent");
    assert_eq!(failed.reported, 0);
    assert_eq!(failed.missing, 1);
    assert!(
        !snapshot.coverage[coverage_index(AgentAttemptOutcome::Cancelled).unwrap()].seen,
        "a coverage cell never recorded stays unavailable"
    );
    assert_eq!(snapshot.unrecognized, 0);
}

#[test]
fn every_known_phase_lands_in_a_distinct_cell() {
    let all = [
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
    let metrics = LocalMetrics::new();
    for (index, phase) in all.iter().enumerate() {
        assert_eq!(
            phase_index(*phase),
            Some(index),
            "{phase:?} has its own slot"
        );
        metrics.record(&AgentMetric::Phase {
            phase: *phase,
            elapsed_ns: index as u64 + 1,
            outcome: AgentMetricOutcome::Succeeded,
        });
    }
    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.unrecognized, 0, "no known variant is unrecognized");
    for (index, phase) in all.iter().enumerate() {
        let landed = cell(&snapshot, *phase);
        assert_eq!(landed.samples, 1, "{phase:?} counted once");
        assert_eq!(
            landed.total_ns,
            index as u64 + 1,
            "{phase:?} has its own cell"
        );
    }
}

#[test]
fn outcomes_are_counted_per_cell() {
    let metrics = LocalMetrics::new();
    let samples = [
        (5_u64, AgentMetricOutcome::Succeeded),
        (7, AgentMetricOutcome::Failed),
        (9, AgentMetricOutcome::Cancelled),
    ];
    for (elapsed_ns, outcome) in samples {
        metrics.record(&AgentMetric::Phase {
            phase: AgentMetricPhase::LocalAttempt,
            elapsed_ns,
            outcome,
        });
    }
    let attempt = cell(&metrics.snapshot(), AgentMetricPhase::LocalAttempt);
    assert_eq!(attempt.samples, 3);
    assert_eq!(attempt.total_ns, 21);
    assert_eq!(attempt.max_ns, 9);
    assert_eq!(
        attempt.outcomes[outcome_index(AgentMetricOutcome::Succeeded).unwrap()],
        1
    );
    assert_eq!(
        attempt.outcomes[outcome_index(AgentMetricOutcome::Failed).unwrap()],
        1
    );
    assert_eq!(
        attempt.outcomes[outcome_index(AgentMetricOutcome::Cancelled).unwrap()],
        1
    );
    assert_eq!(
        attempt.outcomes.iter().sum::<u64>(),
        3,
        "exactly the three samples"
    );
}

#[test]
fn every_known_count_lands_in_a_distinct_slot() {
    let all = [
        AgentMetricCount::RetryAttemptStarted,
        AgentMetricCount::OutputTruncated,
        AgentMetricCount::AdmittedContinuation,
        AgentMetricCount::CompactionStarted,
        AgentMetricCount::ToolResultTruncated,
    ];
    let metrics = LocalMetrics::new();
    for (index, count) in all.iter().enumerate() {
        assert_eq!(count_index(*count), Some(index));
        metrics.record(&AgentMetric::Count(*count));
    }
    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.unrecognized, 0);
    for (index, count) in all.iter().enumerate() {
        assert_eq!(
            snapshot.counts[index], 1,
            "{count:?} counted in its own slot"
        );
    }
}

#[test]
fn the_startup_flag_round_trips() {
    set_enabled(true);
    assert!(enabled());
    set_enabled(false);
    assert!(!enabled());
}
