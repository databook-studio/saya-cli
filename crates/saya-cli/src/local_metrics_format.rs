use crate::local_metrics::{CoverageCell, LocalMetrics, LocalPhaseSnapshot, PhaseCell};
use saya_agent::AgentMetricUseful;
use std::sync::Arc;

/// The one header the section opens with: it names the fixed field contract
/// and declines any benchmark reading up front.
const HEADER: &str = "saya local metrics (fixed fields; not a benchmark)";

/// Duration fields print in whole microseconds: bounded integers, exact at
/// these magnitudes, never a float.
const US: u64 = 1_000;

/// The static label table for the phase slots, aligned with `phase_index`.
const PHASE_LABELS: [&str; 14] = [
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

/// The static tokens for the per-phase outcome counters, aligned with
/// `outcome_index` and kept distinct from the coverage labels.
const OUTCOME_LABELS: [&str; 6] = ["ok", "fail", "cancel", "unavail", "cached", "skipped"];

/// The static label table for the count slots, aligned with `count_index`.
const COUNT_LABELS: [&str; 5] = [
    "retry_attempt_started",
    "output_truncated",
    "admitted_continuation",
    "compaction_started",
    "tool_result_truncated",
];

/// The static label table for the usage-coverage slots, aligned with
/// `coverage_index`.
const COVERAGE_LABELS: [&str; 3] = ["succeeded", "failed", "cancelled"];

/// Renders the fixed snapshot as the diagnostics section: the header, one
/// line per phase slot, one per count slot, one per coverage slot, the
/// first-useful line, and the unrecognized counter. Labels come from a const
/// table aligned with the index functions and numbers are bounded u64s — no
/// string from a request, result, error, or profile can reach this output.
pub(crate) fn format_snapshot(snapshot: &LocalPhaseSnapshot) -> String {
    let mut out = String::from(HEADER);
    out.push('\n');
    for (cell, label) in snapshot.phases.iter().zip(PHASE_LABELS) {
        push_phase(&mut out, label, cell);
    }
    for (count, label) in snapshot.counts.iter().zip(COUNT_LABELS) {
        out.push_str("count ");
        out.push_str(label);
        out.push(' ');
        out.push_str(&count.to_string());
        out.push('\n');
    }
    for (cell, label) in snapshot.coverage.iter().zip(COVERAGE_LABELS) {
        push_coverage(&mut out, label, cell);
    }
    match snapshot.first_useful {
        Some((kind, at_ns)) => {
            out.push_str("first_useful ");
            out.push_str(useful_label(kind));
            out.push_str(" at_us=");
            out.push_str(&(at_ns / US).to_string());
            out.push('\n');
        }
        None => out.push_str("first_useful unavailable\n"),
    }
    out.push_str("unrecognized ");
    out.push_str(&snapshot.unrecognized.to_string());
    out.push('\n');
    out
}

/// Prints the fixed diagnostics section for a settled turn: stderr only,
/// and only when the question carried a collector. The one boundary shared
/// by the `ask` subcommand and the plain REPL's settled turns.
pub(crate) fn print_metrics_section(metrics: &Option<Arc<LocalMetrics>>) {
    if let Some(metrics) = metrics {
        eprintln!("{}", format_snapshot(&metrics.snapshot()));
    }
}

/// One phase line: the bare `unavailable` word for a never-recorded cell, or
/// the sample, duration, and outcome numbers for a recorded one.
fn push_phase(out: &mut String, label: &str, cell: &PhaseCell) {
    out.push_str("phase ");
    out.push_str(label);
    if cell.samples == 0 {
        out.push_str(" unavailable\n");
        return;
    }
    out.push_str(" samples=");
    out.push_str(&cell.samples.to_string());
    out.push_str(" total_us=");
    out.push_str(&(cell.total_ns / US).to_string());
    out.push_str(" max_us=");
    out.push_str(&(cell.max_ns / US).to_string());
    for (slot, label) in cell.outcomes.iter().zip(OUTCOME_LABELS) {
        out.push(' ');
        out.push_str(label);
        out.push('=');
        out.push_str(&slot.to_string());
    }
    out.push('\n');
}

/// One coverage line: the bare `unavailable` word for a never-seen cell, or
/// the reported and missing counters for a seen one.
fn push_coverage(out: &mut String, label: &str, cell: &CoverageCell) {
    out.push_str("coverage ");
    out.push_str(label);
    if !cell.seen {
        out.push_str(" unavailable\n");
        return;
    }
    out.push_str(" reported=");
    out.push_str(&cell.reported.to_string());
    out.push_str(" missing=");
    out.push_str(&cell.missing.to_string());
    out.push('\n');
}

/// The static label for a first-useful kind; a kind added later falls to the
/// fixed `other` token rather than any dynamic string.
fn useful_label(kind: AgentMetricUseful) -> &'static str {
    match kind {
        AgentMetricUseful::NonWhitespaceText => "non_whitespace_text",
        AgentMetricUseful::SuccessfulTool => "successful_tool",
        _ => "other",
    }
}
