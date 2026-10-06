//! Session capacity: the one near-limit notice after a successful save and
//! the friendly refusal when a save is refused at the 4 MiB record cap.
//! Pure wording plus two store-backed helpers the headless loop, the piped
//! REPL, `--turn-file`, and the TUI all share — a notice can never fail a
//! turn, and a limit refusal always names the remedy.

use super::session_resume::block_on;
use super::session_state::SessionState;
use saya_store::{FsSessionStore, SessionCapacityStatus, SessionStore, StoreError};

/// The rotation hint every capacity sentence ends with (no terminator: the
/// sentence adds its own).
pub(crate) const NEAR_LIMIT_HINT: &str =
    "Run /compact, then /rotate to continue in a smaller successor session";

/// The limit refusal's own sentence, before the hint and the resume pointer.
pub(crate) const LIMIT_REACHED: &str =
    "This session's record has reached its 4 MiB limit, so the latest turn could not be saved";

/// The one-time near-limit notice: the stored record's size, one decimal
/// place, beside the 4 MiB ceiling and the remedy.
pub(crate) fn near_limit_text(used_bytes: usize) -> String {
    let mib = used_bytes as f64 / (1024.0 * 1024.0);
    format!("This session's saved record is {mib:.1} MiB of its 4 MiB limit. {NEAR_LIMIT_HINT}.")
}

/// The friendly refusal a save refused at the cap renders, naming the resume
/// path back to the last saved state.
pub(crate) fn limit_reached_text(id: &str) -> String {
    format!("{LIMIT_REACHED}. {NEAR_LIMIT_HINT}; /resume {id} restores the last saved state.")
}

/// The one-time notice a successful save may fire: when the stored record
/// sits at or above the 3 MiB warning level, the sentence is returned once
/// per session process and the flag is set. Anything else — below the level,
/// no record, or an error reading the metadata — says nothing: a check that
/// must never fail a turn.
pub(crate) fn notice_after_save(
    store: &FsSessionStore,
    state: &mut SessionState,
) -> Option<String> {
    if state.capacity_warned {
        return None;
    }
    let capacity = match store.stored_capacity(&state.id) {
        Ok(Some(capacity)) => capacity,
        _ => return None,
    };
    match capacity.status {
        SessionCapacityStatus::Warning | SessionCapacityStatus::OverCap => {
            state.capacity_warned = true;
            Some(near_limit_text(capacity.used_bytes))
        }
        SessionCapacityStatus::Ok => None,
    }
}

/// Why a save failed: refused at the record cap (already worded, with the
/// resume path), or any other store error, unchanged.
#[derive(Debug)]
pub(crate) enum SaveFailure {
    LimitReached(String),
    Other(StoreError),
}

/// The save every headless surface uses: the ordinary store save, with a
/// refusal at the 4 MiB cap mapped to its friendly sentence. Any other error
/// passes through untouched.
pub(crate) fn save_with_explanation(
    store: &FsSessionStore,
    state: &SessionState,
) -> Result<(), SaveFailure> {
    block_on(store.save(state.redacted())).map_err(|error| match error {
        StoreError::LimitExceeded => SaveFailure::LimitReached(limit_reached_text(&state.id)),
        other => SaveFailure::Other(other),
    })
}

#[cfg(test)]
#[path = "session_capacity_tests.rs"]
mod session_capacity_tests;
