//! What one run last wrote at each path, read from its own checkpoint records.
//!
//! Two runs may share one folder, so a restore must not overwrite a file another
//! run changed. The newest checkpoint that records a path holds the digest of
//! the bytes this run left there — or the fact that it left the path absent —
//! and that is the only basis a restore trusts. A path no checkpoint names was
//! never this run's, so a file there is somebody else's.

use super::RewindStore;
use crate::commands::coding::CodingError;
use saya_harness::workspace::Workspace;
use std::collections::HashMap;

/// Every path this run's checkpoints record, with the version its newest
/// checkpoint holds: `Some` digest for a file, `None` for a path it left absent.
pub(super) fn last_written(
    store: &RewindStore,
) -> Result<HashMap<String, Option<String>>, CodingError> {
    let mut last = HashMap::new();
    // The listing is oldest first, so the newest record of a path is the one
    // that stands.
    for checkpoint in store.list()? {
        for entry in checkpoint.entries {
            last.insert(entry.path, entry.sha256);
        }
    }
    Ok(last)
}

/// Whether `path` as it stands is not what this run last wrote there: another
/// run changed it, or wrote it where this run left nothing. Such a path is
/// reported and never overwritten. A file over the rewind's per-file bound is
/// treated the same way — it is never overwritten blind.
pub(super) fn changed_since(
    workspace: &Workspace,
    path: &str,
    last_written: &HashMap<String, Option<String>>,
) -> Result<bool, CodingError> {
    let Some(recorded) = last_written.get(path) else {
        // No checkpoint of this run ever held the path: whatever is there is
        // another run's write.
        return Ok(workspace
            .read(path, super::MAX_REWIND_FILE_BYTES)
            .is_ok_and(|file| !file.truncated));
    };
    match workspace.read(path, super::MAX_REWIND_FILE_BYTES) {
        Ok(file) if file.truncated => Ok(true),
        Ok(file) => Ok(match recorded {
            Some(sha256) => saya_types::sha256_hex(&file.bytes) != *sha256,
            None => true,
        }),
        Err(saya_harness::HarnessError::NotFound { .. }) => Ok(recorded.is_some()),
        Err(error) => Err(CodingError::owner(error)),
    }
}
