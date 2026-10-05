//! Rendering for the saved-session operations: plain text, one decision
//! per line, reusable by any surface that shares the typed outcomes.

use super::{DeleteOutcome, EXIT_SESSIONS_ERROR, RETAINED_NOTE, SessionListing};
use crate::interactive::tui::replay::relative_time;
use saya_store::SessionCapacityStatus;
use std::path::Path;

/// Renders the listing: one line per session (id, size in KiB, capacity
/// status, age), pending deletions named separately, then the storage root
/// as the last line.
pub(crate) fn render_listing(
    listing: &SessionListing,
    now_ms: u128,
    sessions_root: &Path,
) -> String {
    let mut lines: Vec<String> = Vec::new();
    if listing.rows.is_empty() && listing.pending.is_empty() {
        lines.push("No saved sessions.".to_owned());
    }
    for row in &listing.rows {
        let age = relative_time(now_ms.saturating_sub(row.modified_unix_ms));
        let size = format!("{} KiB", row.size_bytes.div_ceil(1024));
        lines.push(format!(
            "{}\t{}\t{}\t{}",
            row.id,
            size,
            capacity_label(row.status),
            age
        ));
    }
    for id in &listing.pending {
        lines.push(format!("pending deletion: {id}"));
    }
    if listing.more {
        lines.push("(more available)".to_owned());
    }
    lines.push(format!("Storage root: {}", sessions_root.display()));
    lines.join("\n")
}

/// The listing label for the store's capacity classification. `over-cap`
/// cannot be produced by a save but is still honest for a record that grew
/// beyond the ceiling out-of-band.
fn capacity_label(status: SessionCapacityStatus) -> &'static str {
    match status {
        SessionCapacityStatus::Ok => "ok",
        SessionCapacityStatus::Warning => "near-limit",
        SessionCapacityStatus::OverCap => "over-cap",
    }
}

/// Renders a delete outcome as (exit code, message). 0 only for a complete
/// delete; every refusal and failure is the usage class.
pub(crate) fn render_delete(outcome: &DeleteOutcome, id: &str) -> (i32, String) {
    match outcome {
        DeleteOutcome::Deleted => (
            0,
            format!("Deleted saved session {id} and its state directory.\n{RETAINED_NOTE}"),
        ),
        DeleteOutcome::Pending { remaining } => (
            EXIT_SESSIONS_ERROR,
            format!(
                "Deleted saved session {id}, but {remaining} paths could not be removed; \
                 run the same command again to retry.\n{RETAINED_NOTE}"
            ),
        ),
        DeleteOutcome::NoSuchSession => (EXIT_SESSIONS_ERROR, format!("no saved session {id}")),
        DeleteOutcome::Live { pid } => (
            EXIT_SESSIONS_ERROR,
            format!("session {id} is live: its lock is held by pid {pid}; nothing was deleted."),
        ),
        DeleteOutcome::InvalidId => (
            EXIT_SESSIONS_ERROR,
            format!("{id:?} is not a valid session id"),
        ),
        DeleteOutcome::CurrentSession => (
            EXIT_SESSIONS_ERROR,
            format!("session {id} is this process's live session; it was not deleted."),
        ),
        DeleteOutcome::Failed(error) => (
            EXIT_SESSIONS_ERROR,
            format!("could not delete saved session {id}: {error}"),
        ),
    }
}
