//! Context-window utilisation thresholds shared by every surface.
//!
//! The project owner set two percentages over the known context window: warn
//! the user at 70%, compact the conversation at 95%. They live here — on the
//! agent contracts — rather than in `saya-cli` or `saya-config` so both the
//! TUI warning (this slice) and a future agent-side compaction trigger read
//! the same numbers without a dependency inversion: `saya-cli` already
//! depends on `saya-agent`, while `saya-agent` depends on neither `saya-cli`
//! nor `saya-config`. A later slice reads the compact threshold; the warning
//! is the only consumer today.

/// Warn the user when context utilisation crosses this percentage of the
/// known window (project-owner decision).
pub const CONTEXT_WARN_PERCENT: u64 = 70;

/// Compact the conversation at this percentage of the known window
/// (project-owner decision). Read by a later slice; declared here so the two
/// thresholds can never drift apart.
pub const CONTEXT_COMPACT_PERCENT: u64 = 95;

/// Context utilisation as a rounded percentage: the last answering call's
/// reported `input_tokens` over the known window. `None` when either side is
/// unknown — absence is not zero, and a zero window is invalid input (config
/// rejects it), never a tiny window — so callers stay silent rather than
/// warn against a fiction.
pub fn context_utilisation_percent(input: Option<u64>, window: Option<u64>) -> Option<u64> {
    let (input, window) = (input?, window?);
    if window == 0 {
        return None;
    }
    Some(((input as f64 / window as f64) * 100.0).round() as u64)
}

#[cfg(test)]
#[path = "context_tests.rs"]
mod tests;
