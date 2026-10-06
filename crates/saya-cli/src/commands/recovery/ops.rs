//! The listing side of recovery: every recorded edit of one session with
//! its current restore status, against the session's checkpoint quotas.
//! Current digests are refreshed for at most [`REFRESH_BOUND`] committed
//! records per call — each read hashes a file of at most 8 MiB — so the
//! listing stays bounded no matter how many edits the session holds.

use super::{BeforeSummary, ChangeItem, ChangesView, StatusNow};
use saya_harness::workspace::Workspace;
use saya_store::FsSessionStore;
use saya_types::{
    CheckpointState, MAX_CHECKPOINT_SESSION_BYTES, MAX_CHECKPOINTS_PER_SESSION, restore_eligibility,
};
use sha2::{Digest, Sha256};

/// At most this many committed records get a fresh current digest per
/// listing; the rest report [`StatusNow::NotChecked`] rather than reading
/// unboundedly.
pub(crate) const REFRESH_BOUND: usize = 32;

/// Lowercase hex sha256 of `bytes` — the integrity check for before-image
/// bytes and the digest helper tests share.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The file's digest as of now: present, missing, or unreadable. An
/// unreadable file is unknown — it is never reported as changed.
enum CurrentDigest {
    Present(String),
    Missing,
    Unreadable,
}

fn current_digest(workspace: Option<&Workspace>, rel: &str) -> CurrentDigest {
    let Some(workspace) = workspace else {
        return CurrentDigest::Unreadable;
    };
    match workspace.read(rel, 0) {
        Ok(read) => CurrentDigest::Present(read.digest),
        Err(error) if error.is_not_found() => CurrentDigest::Missing,
        Err(_) => CurrentDigest::Unreadable,
    }
}

/// Lists every recorded edit of the session, oldest first, each with its
/// current restore status: only committed records are refreshed against
/// the file on disk, at most [`REFRESH_BOUND`] reads per call, and every
/// other record reports its recorded state. Store failures fail the
/// listing; they are never silently dropped.
pub(crate) fn list_changes(
    store: &FsSessionStore,
    session_id: &str,
    workspace: Option<&Workspace>,
) -> Result<ChangesView, String> {
    let usage = store
        .checkpoint_usage(session_id)
        .map_err(|error| error.to_string())?;
    let records = store
        .list_checkpoints(session_id)
        .map_err(|error| error.to_string())?;
    let mut refreshed = 0usize;
    let mut items = Vec::with_capacity(records.len());
    for record in records {
        let restore = if record.state == CheckpointState::Committed {
            if refreshed < REFRESH_BOUND {
                refreshed += 1;
                match current_digest(workspace, &record.path) {
                    CurrentDigest::Present(digest) => {
                        StatusNow::Known(restore_eligibility(&record, Some(&digest)))
                    }
                    CurrentDigest::Missing => StatusNow::Known(restore_eligibility(&record, None)),
                    CurrentDigest::Unreadable => StatusNow::Unreadable,
                }
            } else {
                StatusNow::NotChecked
            }
        } else {
            StatusNow::Known(restore_eligibility(&record, None))
        };
        items.push(ChangeItem {
            id: record.id.as_str().to_owned(),
            path: record.path,
            source: record.source,
            state: record.state,
            before: BeforeSummary::of(&record.before),
            after: record.after,
            created_unix_ms: record.created_unix_ms,
            restore,
        });
    }
    Ok(ChangesView {
        items,
        usage,
        max_edits: MAX_CHECKPOINTS_PER_SESSION,
        max_bytes: MAX_CHECKPOINT_SESSION_BYTES,
    })
}
