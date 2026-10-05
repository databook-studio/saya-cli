//! The shared saved-session operations: bounded listing through the
//! store's own history paging, and deletion through the harness lifecycle
//! owner — the same owner the interactive session holds its lock with.

use super::{DeleteOutcome, LISTING_BOUND, SessionListing, SessionRow};
use saya_harness::HarnessError;
use saya_store::{
    DeleteStatus, FsSessionStore, MAX_SESSION_HISTORY_PAGE_SIZE, SessionHistoryQuery, SessionStore,
    StoreError,
};
use std::path::Path;
use std::{fs::symlink_metadata, time::UNIX_EPOCH};

/// Lists saved sessions newest-first, walking every history page up to
/// [`LISTING_BOUND`] rows; beyond it the outcome says more exist. Sessions
/// with a pending-deletion tombstone are hidden from the rows and named by
/// the store's pending list instead.
pub(crate) async fn list_sessions(store: &FsSessionStore) -> Result<SessionListing, StoreError> {
    let mut rows: Vec<SessionRow> = Vec::new();
    let mut more = false;
    let mut query = SessionHistoryQuery::first_page(MAX_SESSION_HISTORY_PAGE_SIZE)?;
    loop {
        let page = store.history(query.clone()).await?;
        let next_query = query.next_page(&page);
        for entry in page.entries {
            // A record that vanished between the page read and this size
            // check no longer exists; it is not listed.
            let Some(capacity) = store.stored_capacity(&entry.id)? else {
                continue;
            };
            rows.push(SessionRow {
                id: entry.id,
                size_bytes: capacity.used_bytes,
                status: capacity.status,
                modified_unix_ms: entry.modified_unix_ms,
            });
        }
        match next_query {
            Some(_) if rows.len() >= LISTING_BOUND => {
                more = true;
                break;
            }
            Some(next) => query = next,
            None => break,
        }
    }
    let pending = store.pending_deletions()?;
    Ok(SessionListing {
        rows,
        pending,
        more,
    })
}

/// Deletes exactly one saved session through the harness lifecycle owner:
/// its record, state directory, staged temps, and quarantined copies.
/// Nothing is touched for an absent or invalid id, a live session, or the
/// calling session itself; the slash surface passes its own id as
/// `current_session_id` so a session cannot delete itself underneath its
/// owner.
pub(crate) fn delete_saved_session(
    store: &FsSessionStore,
    sessions_root: &Path,
    id: &str,
    current_session_id: Option<&str>,
) -> DeleteOutcome {
    if current_session_id.is_some_and(|current| current == id) {
        return DeleteOutcome::CurrentSession;
    }
    if !is_path_safe(id) {
        return DeleteOutcome::InvalidId;
    }
    // The harness creates `<sessions>/<id>/` transiently when absent, so a
    // typo must be caught first: a session exists only when the store has
    // its record or its state directory. Nothing below runs for an absent
    // id — no directory is conjured, no tombstone is written.
    let exists = match store.stored_capacity(id) {
        Ok(Some(_)) => true,
        Ok(None) => symlink_metadata(sessions_root.join(id))
            .map(|meta| meta.is_dir())
            .unwrap_or(false),
        Err(error) => return DeleteOutcome::Failed(error.to_string()),
    };
    if !exists {
        return DeleteOutcome::NoSuchSession;
    }
    match saya_harness::session_lifecycle::delete_session(store, sessions_root, id) {
        Ok(DeleteStatus::Complete) => DeleteOutcome::Deleted,
        Ok(DeleteStatus::Pending { remaining }) => DeleteOutcome::Pending { remaining },
        Err(HarnessError::LockHeld { pid }) => DeleteOutcome::Live { pid },
        Err(error) => DeleteOutcome::Failed(error.to_string()),
    }
}

/// The store's own id rule (the same shape `session_paths` guards):
/// filename-safe relative names only.
fn is_path_safe(id: &str) -> bool {
    !(id.is_empty() || id.contains('/') || id.contains('\\') || id == "." || id == "..")
}

/// The current unix time in milliseconds, 0 if the clock is before the
/// epoch.
pub(crate) fn unix_ms_now() -> u128 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or(0)
}
