//! What one run last wrote at each path, from its own checkpoint records.
//!
//! Two runs may share one folder, so a restore must not overwrite a file another
//! run changed. The newest checkpoint that records a path holds the digest of
//! the bytes this run left there — or the fact that it left the path absent —
//! and that is the only basis a restore trusts. A path no checkpoint names was
//! never this run's, so a file there is somebody else's.
//!
//! A restore writes bytes too, so it records what it left at each path it
//! touched (`own-writes.json`, one slot, merged): a second rewind into the same
//! run is then compared with the run's own earlier restore instead of with a
//! version it never wrote. The note is a fact about this run's own history,
//! never authority over the folder.

use super::{MAX_REWIND_FILES_PER_CHECKPOINT, MAX_REWIND_RECORD_BYTES, RewindStore};
use crate::commands::coding::{CodingError, context, host};
use saya_harness::workspace::Workspace;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

const VERSION: u32 = 1;
const NAME: &str = "own-writes.json";

/// The note: each path a restore touched, with the digest it left there (`None`
/// for a removal).
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Note {
    version: u32,
    paths: BTreeMap<String, Option<String>>,
}

/// Every path this run's checkpoints record, with the version its newest
/// checkpoint holds: `Some` digest for a file, `None` for a path it left absent.
/// What a restore of this run left at a path stands over any checkpoint, because
/// it is the later write.
pub(super) fn last_written(
    store: &RewindStore,
) -> Result<BTreeMap<String, Option<String>>, CodingError> {
    let mut last = BTreeMap::new();
    // The listing is oldest first, so the newest record of a path is the one
    // that stands.
    for checkpoint in store.list()? {
        for entry in checkpoint.entries {
            last.insert(entry.path, entry.sha256);
        }
    }
    // What a restore of this run left at a path is the later write, so it stands
    // over every checkpoint's version.
    last.extend(store.own_writes());
    Ok(last)
}

/// Whether `path` as it stands is not what this run last wrote there: another
/// run changed it, or wrote it where this run left nothing. Such a path is
/// reported and never overwritten. A file over the rewind's per-file bound is
/// treated the same way — it is never overwritten blind.
pub(super) fn changed_since(
    workspace: &Workspace,
    path: &str,
    last_written: &BTreeMap<String, Option<String>>,
) -> Result<bool, CodingError> {
    let Some(recorded) = last_written.get(path) else {
        // No checkpoint and no restore of this run ever held the path: whatever
        // is there is another run's write.
        return Ok(workspace
            .read(path, super::MAX_REWIND_FILE_BYTES)
            .is_ok_and(|file| !file.truncated));
    };
    match workspace.read(path, super::MAX_REWIND_FILE_BYTES) {
        Ok(file) if file.truncated => Ok(true),
        Ok(file) => {
            let current = saya_types::sha256_hex(&file.bytes);
            Ok(match recorded {
                Some(sha256) => current != *sha256,
                None => true,
            })
        }
        Err(saya_harness::HarnessError::NotFound { .. }) => Ok(recorded.is_some()),
        Err(error) => Err(CodingError::owner(error)),
    }
}

/// Records what one restore left at each path it touched, merged over what
/// earlier restores of this run left.
pub(super) fn record(root: &Path, written: &[(String, Option<String>)]) -> Result<(), CodingError> {
    if written.is_empty() {
        return Ok(());
    }
    let mut paths = read(root);
    for (path, sha256) in written {
        paths.insert(path.clone(), sha256.clone());
    }
    if paths.len() > MAX_REWIND_FILES_PER_CHECKPOINT {
        return Err(CodingError::RewindCaptureTooLarge);
    }
    let note = Note {
        version: VERSION,
        paths,
    };
    let bytes = serde_json::to_vec(&note).map_err(CodingError::owner)?;
    let dir = host::private_directory(root)?;
    let path = dir.join(NAME);
    // One slot: the newest restore's note replaces the previous one, so an
    // unread older note can never outlive it.
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(CodingError::owner(error)),
    }
    context::write_new(&path, &bytes, MAX_REWIND_RECORD_BYTES)
}

/// What earlier restores of this run left at each path. An unreadable note is
/// dropped rather than reported: the checkpoints still hold what each turn left.
pub(super) fn read(root: &Path) -> BTreeMap<String, Option<String>> {
    let Ok(bytes) = context::read(&root.join(NAME), MAX_REWIND_RECORD_BYTES) else {
        return BTreeMap::new();
    };
    match serde_json::from_slice::<Note>(&bytes) {
        Ok(note) if note.version == VERSION => note.paths,
        _ => BTreeMap::new(),
    }
}
