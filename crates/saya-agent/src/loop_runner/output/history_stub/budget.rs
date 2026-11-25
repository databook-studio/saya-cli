//! The history budget and the loop's memory of where a settled region ends.

use super::HISTORY_BUDGET_CEILING_BYTES;

/// Share of a known window, in bytes, the tool-result history may occupy
/// before a pass runs.
const BUDGET_WINDOW_PERCENT: usize = 40;

/// Share of a known window above which even a read may be stubbed.
const HARD_WINDOW_PERCENT: usize = 80;

/// Conventional bytes-per-token estimate, named so the arithmetic is visible.
const BYTES_PER_TOKEN: usize = 4;

/// The loop's own memory of where the settled region ends. A pass rewrites a
/// batch down to the low-water mark and records the last message it touched;
/// later requests append past that point without rewriting the prefix until
/// the budget is exceeded again.
#[derive(Debug, Default)]
pub(in crate::loop_runner) struct HistoryStubState {
    pub(super) settled_through: Option<String>,
}

/// The tool-result byte budget for a known model window: 40 % of the window at
/// four bytes per token, bounded by [`HISTORY_BUDGET_CEILING_BYTES`]. An
/// unknown window uses the fixed ceiling alone.
pub(in crate::loop_runner) fn budget_bytes(window_tokens: Option<u64>) -> usize {
    window_bytes(window_tokens).map_or(HISTORY_BUDGET_CEILING_BYTES, |bytes| {
        (bytes.saturating_mul(BUDGET_WINDOW_PERCENT) / 100).min(HISTORY_BUDGET_CEILING_BYTES)
    })
}

/// The last-resort ceiling: 80 % of a known window, or the fixed ceiling when
/// the window is unknown.
pub(in crate::loop_runner) fn hard_ceiling_bytes(window_tokens: Option<u64>) -> usize {
    window_bytes(window_tokens).map_or(HISTORY_BUDGET_CEILING_BYTES, |bytes| {
        bytes.saturating_mul(HARD_WINDOW_PERCENT) / 100
    })
}

fn window_bytes(window_tokens: Option<u64>) -> Option<usize> {
    let tokens = usize::try_from(window_tokens?).ok()?;
    Some(tokens.saturating_mul(BYTES_PER_TOKEN))
}
