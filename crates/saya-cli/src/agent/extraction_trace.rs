//! Stderr tracing for the post-turn extraction boundary.
//!
//! Without this there is no visibility into the only path that writes
//! memory — no log, no receipt, no counter — and two reviews were misled by the
//! rendered `memory learned ·` line as a result. This module is the durable
//! win: a one-line stderr trace at the `run_extraction` boundary, off by
//! default so production logs nothing. Two ways to turn it on: the
//! `SAYA_EXTRACTION_TRACE` env var, or `--verbose`, which seeds the same gate
//! at startup (see [`enable`]).
//!
//! What is logged — and what is not — is documented on [`trace_extraction`].

use std::sync::OnceLock;

/// Whether the boundary trace prints. Seeded either by [`enable`] (from
/// `--verbose`, before any turn runs) or lazily from the environment.
static ENABLED: OnceLock<bool> = OnceLock::new();

/// Turns the trace on for this process, whatever the environment says.
///
/// Called once at startup when `--verbose` is passed. Seeding the gate is what
/// makes that flag reachable here: the alternative — threading a `verbose:
/// bool` through `run_prompt_with_sink` → `run_prompt_with_inputs`, every test
/// that calls it, and `RuntimeConfig` — is a lot of plumbing for one boolean,
/// and mutating the environment instead is `unsafe` under edition 2024. A
/// later `get_or_init` sees the value already set and does not consult the
/// environment, so the flag wins over an unset variable and agrees with a set
/// one.
pub(crate) fn enable() {
    let _ = ENABLED.set(true);
}

fn enabled() -> bool {
    *ENABLED
        .get_or_init(|| std::env::var_os("SAYA_EXTRACTION_TRACE").is_some_and(|v| !v.is_empty()))
}

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
/// Two triggers, one gate. `SAYA_EXTRACTION_TRACE` matches the existing
/// debug-knob pattern in this crate (`SAYA_HISTORY`, `SAYA_STATE_DB`,
/// `SAYA_SESSION_DIR`) and is safe to leave set in a shell while debugging a
/// "memory didn't record" report. `--verbose` seeds the same gate at startup
/// via [`enable`], which is what a user reaches for first and what that flag
/// previously did not do — it was declared and read nowhere. Off by default,
/// so production logs nothing.
pub(crate) fn trace_extraction(
    outcome: &'static str,
    object_count: usize,
    proposal_count: Option<usize>,
    error: Option<&str>,
    elapsed: Option<std::time::Duration>,
) {
    if !enabled() {
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
    // Duration is what tells you whether EXTRACTION_TIMEOUT is generous or
    // tight against a given gateway. Without it a `timed_out` line says the cap
    // fired but not how close the successful turns were to it, which is the
    // number the cap should be set from.
    let ms = match elapsed {
        Some(d) => format!(" ms={}", d.as_millis()),
        None => String::new(),
    };
    eprintln!("saya extraction: outcome={outcome} objects={object_count}{proposals}{err}{ms}");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `--verbose` calls [`enable`] before any turn runs; the gate must then
    /// report on regardless of the environment. This is the whole point of the
    /// seeding design — the flag was previously declared and read nowhere, so
    /// a test that only exercised the env var would have passed against a dead
    /// flag.
    #[test]
    fn enable_seeds_the_gate_without_touching_the_environment() {
        enable();
        assert!(
            enabled(),
            "--verbose must turn the trace on even with SAYA_EXTRACTION_TRACE unset"
        );
        // Idempotent: a second call is a no-op, not a panic or a reset.
        enable();
        assert!(enabled());
    }
}
