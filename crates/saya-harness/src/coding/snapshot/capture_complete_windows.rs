//! Windows completion verification for published snapshot directories:
//! the same rules as the unix descriptor reader, over paths with
//! reparse-point refusal. Anything missing, malformed, swapped, or
//! over-bound reads as not complete, never as an apparent-complete snapshot.

use std::{
    fs::{self, OpenOptions},
    io::{self, Read as _},
    os::windows::fs::OpenOptionsExt as _,
    path::Path,
};

use saya_types::{Sha256Digest, SnapshotId};

use super::{
    SnapshotError,
    capture_dest::{capture_io, validate_artifact_id},
    manifest,
    types::MAX_SNAPSHOT_MANIFEST_BYTES,
};
use crate::workspace::{contain::FILE_FLAG_OPEN_REPARSE_POINT, windows_identity};

/// The record is exactly the 64-byte lowercase manifest digest `commit`
/// wrote; a longer file is malformed by definition.
pub(super) const MAX_COMPLETION_RECORD_BYTES: usize = 64;
pub(super) const COMPLETION_RECORD: &str = "COMPLETE";
pub(super) const MANIFEST_FILE: &str = "manifest.json";

pub(super) fn is_complete(root: &Path, snapshot_id: &SnapshotId) -> Result<bool, SnapshotError> {
    validate_artifact_id(snapshot_id)?;
    let final_dir = root.join(snapshot_id.as_str());
    let metadata = match fs::symlink_metadata(&final_dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(capture_io("scan final snapshot directory", &error)),
    };
    // A symlink, reparse point, or file holding the id's name is not a
    // snapshot directory.
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Ok(false);
    }
    if windows_identity::path_is_reparse_point(&final_dir)
        .map_err(|error| capture_io("open final snapshot directory", &error))?
    {
        return Ok(false);
    }
    let Some(recorded_bytes) =
        read_optional_bounded(&final_dir, COMPLETION_RECORD, MAX_COMPLETION_RECORD_BYTES)?
    else {
        return Ok(false);
    };
    let Ok(recorded_text) = std::str::from_utf8(&recorded_bytes) else {
        return Ok(false);
    };
    let Ok(recorded) = Sha256Digest::parse(recorded_text.trim_ascii()) else {
        return Ok(false);
    };
    // The recorded digest must match the manifest bytes as they are now.
    let Some(manifest_bytes) =
        read_optional_bounded(&final_dir, MANIFEST_FILE, MAX_SNAPSHOT_MANIFEST_BYTES)?
    else {
        return Ok(false);
    };
    Ok(manifest::digest(&manifest_bytes)? == recorded)
}

/// Reads a final-named file without following links. Absent, symlinked,
/// non-regular, or over-bound files read as `None` (malformed), so a tampered
/// destination can only lower confidence, never fake completion.
pub(super) fn read_optional_bounded(
    dir: &Path,
    name: &str,
    maximum: usize,
) -> Result<Option<Vec<u8>>, SnapshotError> {
    let path = dir.join(name);
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(capture_io("read final snapshot file", &error)),
    };
    if windows_identity::is_reparse_point(&file)
        .map_err(|error| capture_io("scan final snapshot file", &error))?
    {
        return Ok(None);
    }
    // A non-regular entry is never read: the check runs on the opened
    // handle, so a malformed file cannot hang verification.
    if !file
        .metadata()
        .map_err(|error| capture_io("scan final snapshot file", &error))?
        .is_file()
    {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    (&file)
        .take(maximum as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| capture_io("read final snapshot file", &error))?;
    if bytes.len() > maximum {
        return Ok(None);
    }
    Ok(Some(bytes))
}
