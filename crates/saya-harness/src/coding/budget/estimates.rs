//! Estimated physical-attempt usage totals for incomplete usage reports.
//! R04a: an incomplete `reported_usage` is an estimate, not a measurement.

use super::ledger::{State, UsageCoverage};
use saya_agent::TokenUsage;

/// Adds one incomplete physical-attempt usage report to the display-only
/// estimate totals. Estimates are never enforced, never a refund, and never
/// a measured value: the reported totals stay untouched and admission checks
/// never read these fields. On overflow this copies `add_usage`'s overflow
/// path — the state latches `usage_ever_unknown` and usage coverage reads
/// Unknown forever — and leaves the estimate fields unchanged.
pub(super) fn add_estimate(state: &mut State, usage: TokenUsage) {
    let input = state
        .snapshot
        .estimated_input_tokens
        .unwrap_or(0)
        .checked_add(usage.input_tokens);
    let output = state
        .snapshot
        .estimated_output_tokens
        .unwrap_or(0)
        .checked_add(usage.output_tokens);
    let (Some(input), Some(output)) = (input, output) else {
        state.usage_ever_unknown = true;
        state.snapshot.usage_coverage = UsageCoverage::Unknown;
        return;
    };
    state.snapshot.estimated_input_tokens = Some(input);
    state.snapshot.estimated_output_tokens = Some(output);
}
