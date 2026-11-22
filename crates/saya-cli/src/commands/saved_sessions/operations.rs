//! The shared saved-session operations: bounded listing through the
//! store's own history paging, and deletion through the harness lifecycle
//! owner — the same owner the interactive session holds its lock with.

use super::{DeleteOutcome, LISTING_BOUND, SessionListing, SessionRow, run_link};
use crate::commands::coding::job_records;
use saya_harness::HarnessError;
use saya_store::{
    DeleteStatus, FsSessionStore, MAX_SESSION_BYTES, MAX_SESSION_HISTORY_PAGE_SIZE,
    RedactedSession, RunLink, SessionHistoryQuery, SessionStore, StoreError,
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
            // check does not exist; it is not listed.
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
/// its record, state directory, staged temps, and quarantined copies, and —
/// when the record links the session's own finished run — the job that run is
/// on, through [`job_records::delete_records`].
///
/// Nothing is touched for an absent or invalid id, a live session, a job whose
/// folder lease a live process still holds, or the calling session itself; the
/// slash surface passes its own id as `current_session_id` so a session cannot
/// delete itself underneath its owner.
pub(crate) fn delete_saved_session(
    store: &FsSessionStore,
    sessions_root: &Path,
    jobs_root: &Path,
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
    // its record or its state directory — or when its deletion is pending,
    // so the same command can finish it. A tombstone-only leftover (record
    // and dir already gone, tombstone kept because its own removal failed
    // or a crash landed between steps) must stay retryable, never wedged
    // behind "no saved session". Nothing below runs for an absent id with
    // no tombstone — no directory is conjured, no tombstone is written.
    let mut exists = match store.stored_capacity(id) {
        Ok(Some(_)) => true,
        Ok(None) => symlink_metadata(sessions_root.join(id))
            .map(|meta| meta.is_dir())
            .unwrap_or(false),
        Err(error) => return DeleteOutcome::Failed(error.to_string()),
    };
    if !exists {
        match store.pending_deletions() {
            Ok(pending) => exists = pending.iter().any(|pending_id| pending_id == id),
            Err(error) => return DeleteOutcome::Failed(error.to_string()),
        }
    }
    if !exists {
        return DeleteOutcome::NoSuchSession;
    }
    // What the record links is read before anything is removed: a run that is
    // still open refuses the whole delete, and a job that is not the session's
    // to remove is left where it is.
    let link = stored_link(sessions_root, id);
    let facts = link
        .as_ref()
        .map(|link| job_records::facts(jobs_root, &link.job))
        .unwrap_or_default();
    let linked = run_link::decide(link.as_ref(), facts);
    if let run_link::LinkedRun::Refused { job } = &linked {
        return DeleteOutcome::LiveRun {
            job: job.as_str().to_owned(),
        };
    }
    match saya_harness::session_lifecycle::delete_session(store, sessions_root, id) {
        Ok(DeleteStatus::Complete) => {
            remove_linked_job(jobs_root, &linked).unwrap_or(DeleteOutcome::Deleted)
        }
        Ok(DeleteStatus::Pending { remaining }) => {
            remove_linked_job(jobs_root, &linked).unwrap_or(DeleteOutcome::Pending { remaining })
        }
        Err(HarnessError::LockHeld { pid }) => DeleteOutcome::Live { pid },
        Err(error) => DeleteOutcome::Failed(error.to_string()),
    }
}

/// The run link a saved record carries, read before anything is removed, and
/// read plainly: a record the store cannot parse is quarantined by the store's
/// own load, and a delete must never move a record it is about to delete. A
/// record that cannot be read links no job — a delete acts on what it can
/// prove, never on a guess about a job.
fn stored_link(sessions_root: &Path, id: &str) -> Option<RunLink> {
    let path = sessions_root.join(format!("{id}.json"));
    let metadata = symlink_metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_SESSION_BYTES as u64 {
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;
    serde_json::from_slice::<RedactedSession>(&bytes).ok()?.run
}

/// Removes the job a deleted record linked, when the link's decision says so.
/// A job that cannot be removed is a reported failure that states what was
/// already deleted, so the operator is never told nothing happened.
fn remove_linked_job(jobs_root: &Path, linked: &run_link::LinkedRun) -> Option<DeleteOutcome> {
    let run_link::LinkedRun::Delete { job } = linked else {
        return None;
    };
    match job_records::delete_records(jobs_root, job) {
        Ok(()) => Some(DeleteOutcome::Deleted),
        Err(error) => Some(DeleteOutcome::Failed(format!(
            "its job {job} could not be removed: {error}"
        ))),
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
