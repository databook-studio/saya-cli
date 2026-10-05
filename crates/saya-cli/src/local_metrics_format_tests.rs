use crate::local_metrics::LocalMetrics;
use crate::local_metrics_format::format_snapshot;
use saya_agent::{
    AgentAttemptOutcome, AgentMetric, AgentMetricCount, AgentMetricOutcome, AgentMetricPhase,
    AgentMetricUseful,
};

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

/// (a) A fully-fed collector prints every static label exactly once, in the
/// stable phase → count → coverage slot order: the printed slot-label
/// sequences equal the expected tables, proving the label table is aligned
/// with the index functions.
#[test]
fn every_static_label_prints_exactly_once_in_a_stable_order() {
    let output = format_snapshot(&full_snapshot());
    let slot_labels = |kind: &str| -> Vec<&str> {
        output
            .lines()
            .filter_map(|line| line.strip_prefix(kind))
            .filter_map(|rest| rest.split_whitespace().next())
            .collect()
    };
    assert_eq!(
        slot_labels("phase "),
        PHASE_WORDS,
        "phase labels in slot order"
    );
    assert_eq!(
        slot_labels("count "),
        COUNT_WORDS,
        "count labels in slot order"
    );
    assert_eq!(
        slot_labels("coverage "),
        COVERAGE_WORDS,
        "coverage labels in slot order"
    );
    assert!(
        output.starts_with("saya local metrics (fixed fields; not a benchmark)\n"),
        "the output opens with the fixed header: {output}"
    );
    assert!(
        output.contains("first_useful non_whitespace_text"),
        "the kept first-useful kind is named: {output}"
    );
    assert!(
        !output.contains("successful_tool"),
        "only the first useful record is kept: {output}"
    );
    assert!(
        output
            .lines()
            .any(|line| line.starts_with("unrecognized 0")),
        "the unrecognized counter is printed: {output}"
    );
}

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

/// (d) The whole output is made only of the allowed label vocabulary, digits,
/// and punctuation: no request, result, error, or profile string can reach the
/// formatter by construction, and this check fails if that ever changes.
#[test]
fn the_output_is_only_labels_digits_and_punctuation() {
    const STRUCTURE: [&str; 25] = [
        "saya",
        "local",
        "metrics",
        "fixed",
        "fields",
        "not",
        "a",
        "benchmark",
        "phase",
        "count",
        "coverage",
        "first_useful",
        "unrecognized",
        "unavailable",
        "samples",
        "total_us",
        "max_us",
        "ok",
        "fail",
        "cancel",
        "unavail",
        "cached",
        "skipped",
        "reported",
        "missing",
    ];
    const KINDS: [&str; 3] = ["at_us", "non_whitespace_text", "successful_tool"];
    let vocabulary: Vec<&str> = PHASE_WORDS
        .into_iter()
        .chain(COUNT_WORDS)
        .chain(COVERAGE_WORDS)
        .chain(STRUCTURE)
        .chain(KINDS)
        .collect();
    let output = format_snapshot(&full_snapshot());
    assert!(
        output
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || " _=(;)\n".contains(c)),
        "only lowercase labels, digits, and known punctuation: {output}"
    );
    for token in output.split_whitespace() {
        let letters: String = token
            .chars()
            .filter(|c| c.is_ascii_lowercase() || *c == '_')
            .collect();
        assert!(
            letters.is_empty() || vocabulary.contains(&letters.as_str()),
            "every token is vocabulary or a number: {token:?} in {output}"
        );
    }
}
