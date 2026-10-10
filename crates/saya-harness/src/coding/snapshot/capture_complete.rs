//! Completion verification for published snapshot directories. Only a
//! final-named directory holding a `COMPLETE` record whose digest matches
//! the bytes of its `manifest.json` is complete; anything missing,
//! malformed, or swapped — including symlinked files — reads as not
//! complete, never as an apparent-complete snapshot. The whole module is
//! supported-target only: `SnapshotArtifacts::is_complete` refuses on the
//! others and never reaches this code.

#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::{ffi::OsStr, fs, io, os::fd::AsRawFd};

#[cfg(any(target_os = "macos", target_os = "linux"))]
use saya_types::{Sha256Digest, SnapshotId};

#[cfg(any(target_os = "macos", target_os = "linux"))]
use super::{
    SnapshotError, capture_dest::capture_io, manifest, types::MAX_SNAPSHOT_MANIFEST_BYTES,
};

#[cfg(any(target_os = "macos", target_os = "linux"))]
use crate::workspace::fd::{open_dir_at, open_file_at, stat_at};

/// The record is exactly the 64-byte lowercase manifest digest `commit`
/// wrote; a longer file is malformed by definition.
#[cfg(any(target_os = "macos", target_os = "linux"))]
const MAX_COMPLETION_RECORD_BYTES: usize = 64;
#[cfg(any(target_os = "macos", target_os = "linux"))]
const COMPLETION_RECORD: &str = "COMPLETE";
#[cfg(any(target_os = "macos", target_os = "linux"))]
const MANIFEST_FILE: &str = "manifest.json";

/// Re-checks `snapshot_id` at the artifacts boundary, then verifies the
/// final directory's completion record against its manifest bytes.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(super) fn is_complete(
    root_dir: &fs::File,
    snapshot_id: &SnapshotId,
) -> Result<bool, SnapshotError> {
    super::capture_dest::validate_artifact_id(snapshot_id)?;
    is_complete_unix(root_dir, snapshot_id)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn is_complete_unix(root_dir: &fs::File, snapshot_id: &SnapshotId) -> Result<bool, SnapshotError> {
    let id_name = OsStr::new(snapshot_id.as_str());
    let final_stat = match stat_at(root_dir.as_raw_fd(), id_name) {
        Ok(stat) => stat,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(capture_io("scan final snapshot directory", &error)),
    };
    // A symlink or file holding the id's name is not a snapshot directory.
    if !final_stat.is_dir() {
        return Ok(false);
    }
    let final_dir = match open_dir_at(root_dir.as_raw_fd(), id_name) {
        Ok(dir) => fs::File::from(dir),
        Err(error)
            if error.kind() == io::ErrorKind::NotFound
                || error.raw_os_error() == Some(libc::ELOOP) =>
        {
            return Ok(false);
        }
        Err(error) => return Err(capture_io("open final snapshot directory", &error)),
    };
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

/// Reads a final-named file without following links. The open is
/// nonblocking, so a FIFO at the name cannot block it, and the opened
/// descriptor's own type decides: absent, symlinked, non-regular, or
/// over-bound files read as `None` (malformed), so a tampered destination
/// can only lower confidence, never fake completion.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn read_optional_bounded(
    dir: &fs::File,
    name: &str,
    maximum: usize,
) -> Result<Option<Vec<u8>>, SnapshotError> {
    use std::io::Read as _;
    let file = match open_file_at(dir.as_raw_fd(), OsStr::new(name), libc::O_NONBLOCK, 0) {
        Ok(file) => file,
        Err(error)
            if error.kind() == io::ErrorKind::NotFound
                || error.raw_os_error() == Some(libc::ELOOP) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(capture_io("read final snapshot file", &error)),
    };
    // A FIFO (or any non-regular entry) is never read: the check runs on
    // the opened descriptor, so a malformed file cannot hang verification.
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
