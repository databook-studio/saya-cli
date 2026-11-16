//! Windows read-back of a published snapshot: the id-named final directory
//! is opened by path with reparse-point refusal, both records are read once
//! through the bounded no-follow helper, and the shared strict parse verifies
//! them.

use std::{fs, io, path::Path};

use saya_types::SnapshotId;

use super::{
    CapturedSnapshot, SnapshotError,
    capture_complete_windows::{
        COMPLETION_RECORD, MANIFEST_FILE, MAX_COMPLETION_RECORD_BYTES, read_optional_bounded,
    },
    capture_dest::capture_io,
    capture_manifest_read::parse_verified,
    types::MAX_SNAPSHOT_MANIFEST_BYTES,
};
use crate::workspace::windows_identity;

pub(super) fn read(
    root: &Path,
    snapshot_id: &SnapshotId,
) -> Result<CapturedSnapshot, SnapshotError> {
    let final_dir = root.join(snapshot_id.as_str());
    let metadata = match fs::symlink_metadata(&final_dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(SnapshotError::SnapshotNotFound);
        }
        Err(error) => return Err(capture_io("scan final snapshot directory", &error)),
    };
    // A symlink, reparse point, or file holding the id's name is not a
    // snapshot directory.
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(SnapshotError::CapturedManifestMalformed);
    }
    if windows_identity::path_is_reparse_point(&final_dir)
        .map_err(|error| capture_io("open final snapshot directory", &error))?
    {
        return Err(SnapshotError::CapturedManifestMalformed);
    }
    // Both records are read once, bounded; any absence, symlinked,
    // non-regular, or over-bound entry is malformed, never a snapshot.
    let Some(recorded_bytes) =
        read_optional_bounded(&final_dir, COMPLETION_RECORD, MAX_COMPLETION_RECORD_BYTES)?
    else {
        return Err(SnapshotError::CapturedManifestMalformed);
    };
    let Some(manifest_bytes) =
        read_optional_bounded(&final_dir, MANIFEST_FILE, MAX_SNAPSHOT_MANIFEST_BYTES)?
    else {
        return Err(SnapshotError::CapturedManifestMalformed);
    };
    parse_verified(&recorded_bytes, &manifest_bytes, snapshot_id)
}
