//! The one classification table for reload planning, keyed by the diagnostics
//! field names. Authority-sensitive settings are never applied to a running
//! session, and an unknown field defaults to pending (safe). A new
//! `ResolvedDiagnostics` field must land here consciously — the drift guard
//! in `reload_tests.rs` fails otherwise.

use super::plan::{ChangeLine, ReloadPlan};
use crate::config::runtime::RuntimeConfig;
use crate::interactive::SessionState;

/// What a changed setting may do to a running session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Class {
    Live,
    /// A session-level copy shadows the runtime value; re-seed only when the
    /// user never overrode it.
    Reseedable(ReseedTarget),
    Authority,
    NextSession,
    /// Not in the table: pending is the safe default.
    Unrecognised,
}

/// Which session-level copy a reseedable key shadows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReseedTarget {
    Provider,
    Model,
    Profile,
    Endpoint,
}

pub(crate) const CLASS_TABLE: &[(&str, Class)] = &[
    ("allow_data_sharing", Class::Authority),
    ("api_key_reference", Class::Live),
    ("base_url", Class::Reseedable(ReseedTarget::Endpoint)),
    ("candidates", Class::Live),
    // Coding settings are read when a coding job is created or resumed —
    // never by the running REPL — and `/reload` swaps the runtime handle
    // before the next line, so the next job in this session observes them:
    // live, with nothing pending and no restart owed.
    ("coding_check_profiles", Class::Live),
    ("coding_enabled", Class::Live),
    ("coding_extensions", Class::Live),
    ("coding_max_children", Class::Live),
    ("coding_max_concurrent_commands", Class::Live),
    ("coding_max_concurrent_model_requests", Class::Live),
    ("coding_max_elapsed_seconds", Class::Live),
    ("coding_max_job_tokens", Class::Live),
    ("coding_max_known_tokens", Class::Live),
    ("coding_max_logical_model_requests", Class::Live),
    ("coding_max_physical_transport_attempts", Class::Live),
    ("coding_max_tool_calls", Class::Live),
    ("compaction", Class::Live),
    ("context_byte_budget", Class::Live),
    ("context_window_tokens", Class::Live),
    ("endpoints", Class::Live),
    ("idle_timeout_seconds", Class::Live),
    ("jobs_runner_allow", Class::NextSession),
    ("jobs_runner_program_dir", Class::NextSession),
    ("jobs_runner_timeout_seconds", Class::NextSession),
    ("jobs_tokens_per_endpoint", Class::Live),
    ("jobs_tool_calls", Class::Live),
    ("jobs_turns", Class::Live),
    ("jobs_wall_clock_seconds", Class::Live),
    ("max_iterations", Class::Live),
    ("max_output_tokens", Class::Live),
    ("max_rows", Class::Live),
    ("memory_max_claims_per_contract", Class::Live),
    ("memory_max_contracts", Class::Live),
    ("memory_max_context_bytes", Class::Live),
    ("memory_mode", Class::Live),
    ("model", Class::Reseedable(ReseedTarget::Model)),
    ("output_color", Class::NextSession),
    ("profile_dialect", Class::NextSession),
    ("profile_name", Class::Reseedable(ReseedTarget::Profile)),
    ("provider", Class::Reseedable(ReseedTarget::Provider)),
    ("query_timeout_seconds", Class::Live),
    ("read_only", Class::Live),
    ("retry_delays_ms", Class::Live),
    ("show_thinking", Class::NextSession),
    ("temperature", Class::Live),
    ("timeout_seconds", Class::Live),
    ("ui_theme", Class::NextSession),
];

/// Unlisted fields default to pending; this documents which that is today.
/// Test-only: its one consumer is the drift guard in `reload_tests.rs`, and
/// a documentation constant that no production path reads would otherwise be
/// dead code now that the module compiles ungated.
#[cfg(test)]
pub(crate) const DEFAULTS_TO_PENDING: &[&str] = &["output_format"];

const OVERRIDE_NOTE: &str =
    "this session overrides it (use /provider, /model or /connect to change it here)";
const AUTHORITY_NOTE: &str = "data sharing is fixed for this session; use /privacy to change it here, or start a new session";
const UNRECOGNISED_NOTE: &str = "unrecognised setting; it takes effect in a new session";
pub(crate) const NEXT_SESSION_NOTE: &str = "takes effect in a new session";

pub(crate) fn classify_key(key: &str) -> Class {
    CLASS_TABLE
        .iter()
        .find(|(name, _)| *name == key)
        .map_or(Class::Unrecognised, |(_, class)| *class)
}

/// Route one changed diagnostics key into the plan.
pub(crate) fn classify_change(
    change: &ChangeLine,
    old: &RuntimeConfig,
    new: &RuntimeConfig,
    state: &SessionState,
    plan: &mut ReloadPlan,
) {
    match classify_key(&change.key) {
        Class::Live => plan.applied.push(change.clone()),
        Class::Authority => plan.pending.push(with_note(change, AUTHORITY_NOTE)),
        Class::NextSession => plan.pending.push(with_note(change, NEXT_SESSION_NOTE)),
        Class::Unrecognised => plan.pending.push(with_note(change, UNRECOGNISED_NOTE)),
        Class::Reseedable(target) => {
            if target.already_matches(new, state) {
                return;
            }
            // Untouched ⇔ the state's current value equals the old runtime's
            // value; then the new value is re-seeded. If the new value already
            // equals the state's current value, nothing is needed.
            if target.user_overrode(old, state) {
                plan.pending.push(with_note(change, OVERRIDE_NOTE));
            } else {
                plan.reseeded.push(change.clone());
                plan.reseed.write(target, new);
            }
        }
    }
}

fn with_note(change: &ChangeLine, note: &str) -> ChangeLine {
    let mut line = change.clone();
    line.note = Some(note.to_owned());
    line
}
