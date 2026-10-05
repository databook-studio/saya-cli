//! Reading checkpoint data back: bounded record loads that re-check every
//! gate and the record's identity, before-image reads verified against the
//! recorded digest and size, verified read access, and the removal one
//! checkpoint's files. Nothing here deletes anything but an explicitly
//! requested checkpoint, and no bytes ever leave unverified.

use super::files::{
    dir_path, image_path, record_path, remove_checkpoint_files, sha256_hex, write_record,
};
use super::{CheckpointStoreError, invalid};
use crate::filesystem::is_valid_id;
use crate::private_file::{bounded_read, io_error};
use crate::{FsSessionStore, StoreError};
use saya_types::{BeforeImage, CheckpointId, EditCheckpoint, MAX_CHECKPOINT_RECORD_BYTES};
use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

/// Loads the record behind `id` for a lifecycle transition: the session
/// gate, the id shape, then the file itself under its own identity checks.
pub(super) fn loaded_record(
    store: &FsSessionStore,
    session_id: &str,
    id: &str,
) -> Result<(PathBuf, EditCheckpoint), CheckpointStoreError> {
    super::gated_session(store, session_id)?;
    let cid = CheckpointId::parse(id).map_err(|_| CheckpointStoreError::NotFound)?;
    let dir = dir_path(store.sessions_root(), session_id);
    let record = read_record_file(&record_path(&dir, id), &cid, session_id)?;
    Ok((dir, record))
}

/// Stamps `updated` (never before `created`), re-validates, and rewrites
/// the record atomically.
pub(super) fn rewrite(
    dir: &Path,
    id: &str,
    record: &mut EditCheckpoint,
    now_unix_ms: i64,
) -> Result<EditCheckpoint, CheckpointStoreError> {
    record.updated_unix_ms = now_unix_ms.max(record.created_unix_ms);
    let bytes = record.to_json_bytes().map_err(|_| invalid())?;
    write_record(dir, id, &bytes)?;
    Ok(record.clone())
}

/// Loads one record file through every gate: symlink refusal, bounded
/// read, strict parse with validation, and the record's own id and session
/// must match the file name and directory it was found under. Absent →
/// `NotFound`; a symlink, oversize, unparsable, or identity-mismatched file
/// → `Corrupt`.
pub(super) fn read_record_file(
    path: &Path,
    id: &CheckpointId,
    session_id: &str,
) -> Result<EditCheckpoint, CheckpointStoreError> {
    let meta = fs::symlink_metadata(path).map_err(|error| match error.kind() {
        ErrorKind::NotFound => CheckpointStoreError::NotFound,
        _ => CheckpointStoreError::from(io_error(error)),
    })?;
    if meta.file_type().is_symlink() {
        return Err(CheckpointStoreError::Corrupt);
    }
    let bytes = bounded_read(path, MAX_CHECKPOINT_RECORD_BYTES)
        .map_err(|error| match error {
            StoreError::LimitExceeded => CheckpointStoreError::Corrupt,
            other => CheckpointStoreError::from(other),
        })?
        .ok_or(CheckpointStoreError::NotFound)?;
    let record =
        EditCheckpoint::from_json_bytes(&bytes).map_err(|_| CheckpointStoreError::Corrupt)?;
    if record.id != *id || record.session_id != session_id {
        return Err(CheckpointStoreError::Corrupt);
    }
    Ok(record)
}

/// Reads one before-image and verifies it against its record: present,
/// regular (never through a link), at most the recorded size, and hashing
/// exactly to the recorded digest — any mismatch is `Corrupt`, and no
/// unverified bytes ever leave this function. An `Absent` before-image has
/// no file to read.
pub(super) fn read_image(
    dir: &Path,
    id: &str,
    record: &EditCheckpoint,
) -> Result<Vec<u8>, CheckpointStoreError> {
    let BeforeImage::Present(version) = &record.before else {
        return Err(CheckpointStoreError::NotFound);
    };
    let path = image_path(dir, id);
    let meta = fs::symlink_metadata(&path).map_err(|error| match error.kind() {
        ErrorKind::NotFound => CheckpointStoreError::Corrupt,
        _ => CheckpointStoreError::from(io_error(error)),
    })?;
    if meta.file_type().is_symlink() {
        return Err(CheckpointStoreError::Corrupt);
    }
    let max = usize::try_from(version.size).map_err(|_| CheckpointStoreError::Corrupt)?;
    let bytes = bounded_read(&path, max)
        .map_err(|error| match error {
            StoreError::LimitExceeded => CheckpointStoreError::Corrupt,
            other => CheckpointStoreError::from(other),
        })?
        .ok_or(CheckpointStoreError::Corrupt)?;
    if bytes.len() as u64 != version.size || sha256_hex(&bytes) != version.sha256 {
        return Err(CheckpointStoreError::Corrupt);
    }
    Ok(bytes)
}

impl FsSessionStore {
    /// The session's exact before-image bytes for `id`, verified against
    /// the record's digest and size: mismatched or missing bytes are
    /// `Corrupt`, and a record without a stored image (`Absent`) has none
    /// to read.
    pub fn read_before_image(
        &self,
        session_id: &str,
        id: &str,
    ) -> Result<Vec<u8>, CheckpointStoreError> {
        if !is_valid_id(session_id) {
            return Err(invalid());
        }
        let cid = CheckpointId::parse(id).map_err(|_| CheckpointStoreError::NotFound)?;
        let dir = dir_path(self.sessions_root(), session_id);
        let record = read_record_file(&record_path(&dir, id), &cid, session_id)?;
        read_image(&dir, id, &record)
    }

    /// Removes one checkpoint's record and image, record first so a crash
    /// between the two leaves an orphan image that recovery removes.
    /// Idempotent; unknown or malformed ids are `NotFound`.
    pub fn delete_checkpoint(
        &self,
        session_id: &str,
        id: &str,
    ) -> Result<(), CheckpointStoreError> {
        if !is_valid_id(session_id) {
            return Err(invalid());
        }
        CheckpointId::parse(id).map_err(|_| CheckpointStoreError::NotFound)?;
        remove_checkpoint_files(&dir_path(self.sessions_root(), session_id), id)
    }
}
