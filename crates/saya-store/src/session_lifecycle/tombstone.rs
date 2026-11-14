//! The durable tombstone for a session being deleted: a small versioned
//! record at `<state>/lifecycle/session-<id>.tombstone` — outside the
//! removable `<sessions>` tree — written BEFORE anything is deleted, so a
//! crash at any later point leaves a truthful record (the setup commit
//! marker in saya-cli is the precedent). The tombstone makes the session
//! invisible at once, refuses saves of it, and is removed only after every
//! removal has succeeded, so a retry converges. Reading one is strict:
//! corrupt, oversize, or self-inconsistent content fails closed — a pending
//! deletion is never ignored.

use crate::StoreError;
use crate::filesystem::{is_valid_id, stamp};
use crate::private_file::{bounded_read, io_error, stage_and_publish};
use crate::replace::AtomicReplace;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

#[cfg(unix)]
use crate::private_file::set_mode;

/// Any tombstone is a few dozen bytes; 4 KiB bounds every read of one.
pub(crate) const TOMBSTONE_MAX_BYTES: usize = 4 * 1024;
/// The tombstone's extension, outside `<sessions>` so the `*.json` id scans
/// never see it and a tombstone is never mis-listed as a session.
pub(crate) const TOMBSTONE_EXTENSION: &str = "tombstone";
const TOMBSTONE_PREFIX: &str = "session-";
const TOMBSTONE_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TombstoneV1 {
    version: u32,
    id: String,
    started_unix_ms: u64,
}

/// `<state>/lifecycle/`, derived from the store root's parent. A root with
/// no parent has no state directory to hold a tombstone — refuse.
pub(crate) fn lifecycle_dir(store_root: &Path) -> Result<PathBuf, StoreError> {
    let state = store_root.parent().ok_or_else(StoreError::unavailable)?;
    Ok(state.join("lifecycle"))
}

/// The tombstone path for `id` — the caller has validated it first.
pub(crate) fn path(store_root: &Path, id: &str) -> Result<PathBuf, StoreError> {
    Ok(lifecycle_dir(store_root)?.join(format!("{TOMBSTONE_PREFIX}{id}.{TOMBSTONE_EXTENSION}")))
}

/// Publishes the tombstone — private (0600), fsynced, atomic rename.
/// Idempotent: a rewrite replaces the record with a fresh start stamp.
pub(crate) fn write(store_root: &Path, id: &str) -> Result<(), StoreError> {
    let dir = lifecycle_dir(store_root)?;
    fs::create_dir_all(&dir).map_err(io_error)?;
    #[cfg(unix)]
    set_mode(&dir, 0o700)?;
    let record = TombstoneV1 {
        version: TOMBSTONE_VERSION,
        id: id.to_owned(),
        started_unix_ms: u64::try_from(stamp()).unwrap_or(u64::MAX),
    };
    let bytes = serde_json::to_vec_pretty(&record).map_err(|_| StoreError::unavailable())?;
    stage_and_publish(&path(store_root, id)?, &bytes, &AtomicReplace)
}

/// Whether a VALID tombstone exists for `id`: `false` when absent, `true`
/// when present and well-formed. Corrupt, oversize, version-unknown, or
/// name-mismatched content fails closed with [`StoreError::Invalid`] —
/// never ignored, never guessed. A symlink planted at the name is likewise
/// refused, never read through.
pub(crate) fn exists(store_root: &Path, id: &str) -> Result<bool, StoreError> {
    let path = path(store_root, id)?;
    let meta = match fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(io_error(error)),
    };
    if meta.file_type().is_symlink() {
        return Err(StoreError::Invalid);
    }
    let bytes = match bounded_read(&path, TOMBSTONE_MAX_BYTES) {
        Ok(Some(value)) => value,
        Ok(None) => return Ok(false),
        Err(StoreError::LimitExceeded) => return Err(StoreError::Invalid),
        Err(error) => return Err(error),
    };
    let record: TombstoneV1 = serde_json::from_slice(&bytes).map_err(|_| StoreError::Invalid)?;
    if record.version != TOMBSTONE_VERSION || record.id != id {
        return Err(StoreError::Invalid);
    }
    Ok(true)
}

/// Every id with a tombstone present, sorted — the observable set of
/// pending deletions. Names that do not parse to a valid session id are
/// skipped: saves refuse such ids, so they name no real session.
pub(crate) fn deleted_ids(store_root: &Path) -> Result<Vec<String>, StoreError> {
    let dir = lifecycle_dir(store_root)?;
    let entries = match fs::read_dir(&dir) {
        Ok(value) => value,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err(StoreError::unavailable()),
    };
    let suffix = format!(".{TOMBSTONE_EXTENSION}");
    let mut ids = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let Some(name) = entry.file_name().into_string().ok() else {
            continue;
        };
        if let Some(id) = name
            .strip_prefix(TOMBSTONE_PREFIX)
            .and_then(|rest| rest.strip_suffix(&suffix))
            .filter(|id| is_valid_id(id))
        {
            ids.push(id.to_owned());
        }
    }
    ids.sort_unstable();
    Ok(ids)
}
