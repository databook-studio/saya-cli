//! Stderr tracing for the post-turn extraction boundary (spec packet-54 open
//! question).
//!
//! Until this slice there was zero visibility into the only path that writes
//! memory — no log, no receipt, no counter — and two reviews were misled by the
//! rendered `memory learned ·` line as a result. This module is the durable
//! win: a one-line stderr trace at the `run_extraction` boundary, gated by an
//! env var so production logs nothing by default.
//!
//! What is logged — and what is not — is documented on [`trace_extraction`].

use std::sync::OnceLock;

/// Traces one post-turn extraction boundary event to stderr when
/// `SAYA_EXTRACTION_TRACE` is set (any non-empty value).
///
/// Logged:
/// - the outcome token (`ok` / `failed` / `timed_out` / `gate_declined`),
/// - the number of objects in the turn's object table,
/// - the count of proposals persisted (when extraction ran), and
/// - on error the `ExtractionRunnerError` display string (a category + short
///   message, not user data).
///
/// Never logged: the raw model response. It may contain user data (rows,
/// prompts), so it is excluded by construction — `run_extraction` does not
/// surface it here, and this trace does not print it even when enabled.
///
/// Trigger choice — env var, not `--verbose`: `--verbose` (`cli.rs:39`) is a
/// dead flag today (no read site anywhere in the workspace), so reaching it at
/// this seam would mean threading a new `verbose: bool` through
/// `run_prompt_with_sink` → `run_prompt_with_inputs` and every test that calls
/// the latter, plus a `RuntimeConfig` field — invasive plumbing for a knob
/// that is wired to nothing yet. An env var matches the existing debug-knob
/// pattern in this crate (`SAYA_HISTORY`, `SAYA_STATE_DB`, `SAYA_SESSION_DIR`),
/// needs no plumbing, is read once and cached, and is safe to leave set in a
/// shell where a user is debugging a "memory didn't record" report. Off by
/// default, so production logs nothing.
pub(crate) fn trace_extraction(
    outcome: &'static str,
    object_count: usize,
    proposal_count: Option<usize>,
    error: Option<&str>,
) {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    let enabled = *ENABLED
        .get_or_init(|| std::env::var_os("SAYA_EXTRACTION_TRACE").is_some_and(|v| !v.is_empty()));
    if !enabled {
        return;
    }
    let proposals = match proposal_count {
        Some(n) => format!(" proposals={n}"),
        None => String::new(),
    };
    let err = match error {
        Some(e) => format!(" error={e}"),
        None => String::new(),
    };
    eprintln!("saya extraction: outcome={outcome} objects={object_count}{proposals}{err}");
}
