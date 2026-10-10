//! Strict, verified reader of a published snapshot: the id-named final
//! directory `commit` published holds `manifest.json` plus a `COMPLETE`
//! record whose digest must match the manifest bytes as they are now. This
//! module reads that state back through no-follow, nonblocking, bounded
//! reads and rebuilds a `CapturedSnapshot`, refusing every malformed,
//! swapped, or out-of-bounds alternative as a typed error. Object bytes
//! are not read here: the materializer re-hashes each object while
//! copying.

use saya_types::SnapshotId;

use super::{
    CapturedSnapshot, SnapshotArtifacts, SnapshotError, capture_dest::validate_artifact_id,
};

#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::{collections::HashSet, ffi::OsStr, fs, io, os::fd::AsRawFd};

#[cfg(any(target_os = "macos", target_os = "linux"))]
use saya_types::{GitObjectId, RepositoryId, Sha256Digest};

#[cfg(any(target_os = "macos", target_os = "linux"))]
use serde::Deserialize;

#[cfg(any(target_os = "macos", target_os = "linux"))]
use super::{
    CapturedEntry, SnapshotContentSource, SnapshotFileMode, SnapshotPath,
    capture_complete::{
        COMPLETION_RECORD, MANIFEST_FILE, MAX_COMPLETION_RECORD_BYTES, read_optional_bounded,
    },
    capture_dest::capture_io,
    manifest,
    types::{
        MAX_SNAPSHOT_FILE_BYTES, MAX_SNAPSHOT_FILES, MAX_SNAPSHOT_MANIFEST_BYTES,
        MAX_SNAPSHOT_TOTAL_BYTES,
    },
};

#[cfg(any(target_os = "macos", target_os = "linux"))]
use crate::workspace::fd::{open_dir_at, stat_at};

/// The stored content source: the same internally tagged shape the
/// capture orchestrator serialized, parsed strictly. `WorkingTree` is an
/// empty STRUCT variant — not a unit variant — so `deny_unknown_fields`
/// applies to it; a unit variant would silently ignore extra map
/// entries.
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StoredSource {
    GitBlob { object_id: GitObjectId },
    WorkingTree {},
}

/// One stored entry: exactly the fields, in the same contract order, the
/// capture orchestrator serialized. The parse is strict — unknown fields
/// are malformed — and `path` re-validates itself on read.
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredEntry {
    path: SnapshotPath,
    mode: SnapshotFileMode,
    content_sha256: Sha256Digest,
    captured_bytes: u64,
    source: StoredSource,
}

/// The stored captured manifest: the same fixed field order the capture
/// orchestrator wrote, parsed strictly.
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredManifest {
    schema_version: u32,
    snapshot_id: SnapshotId,
    repository_id: RepositoryId,
    plan_manifest_sha256: Sha256Digest,
    entries: Vec<StoredEntry>,
}

impl SnapshotArtifacts {
    /// Reads back and VERIFIES one published snapshot: ONE read of
    /// `COMPLETE` and one of `manifest.json`, both through no-follow
    /// nonblocking bounded reads. `digest(manifest bytes)` must equal the
    /// recorded digest; the parse is strict (`deny_unknown_fields`);
    /// `schema_version` must be 1; the id inside the manifest must equal
    /// the requested id; the entry count must stay within
    /// `MAX_SNAPSHOT_FILES`; each `captured_bytes` within
    /// `MAX_SNAPSHOT_FILE_BYTES`; their sum within
    /// `MAX_SNAPSHOT_TOTAL_BYTES` with checked addition; and all paths
    /// unique — each path is re-validated by `SnapshotPath`'s own
    /// `Deserialize`. It does NOT verify object bytes: the materializer
    /// re-hashes each object while copying.
    pub fn read_snapshot(
        &self,
        snapshot_id: &SnapshotId,
    ) -> Result<CapturedSnapshot, SnapshotError> {
        validate_artifact_id(snapshot_id)?;
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            read_unix(self.root_dir(), snapshot_id)
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            let _ = snapshot_id;
            Err(SnapshotError::ArtifactRootRefused)
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn read_unix(
    root_dir: &fs::File,
    snapshot_id: &SnapshotId,
) -> Result<CapturedSnapshot, SnapshotError> {
    let id_name = OsStr::new(snapshot_id.as_str());
    let final_stat = match stat_at(root_dir.as_raw_fd(), id_name) {
        Ok(stat) => stat,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(SnapshotError::SnapshotNotFound);
        }
        Err(error) => return Err(capture_io("scan final snapshot directory", &error)),
    };
    // A symlink or file holding the id's name is not a snapshot directory.
    if !final_stat.is_dir() {
        return Err(SnapshotError::CapturedManifestMalformed);
    }
    let final_dir = match open_dir_at(root_dir.as_raw_fd(), id_name) {
        Ok(dir) => fs::File::from(dir),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(SnapshotError::SnapshotNotFound);
        }
        Err(error)
            if matches!(
                error.raw_os_error(),
                Some(libc::ELOOP) | Some(libc::ENOTDIR)
            ) =>
        {
            return Err(SnapshotError::CapturedManifestMalformed);
        }
        Err(error) => return Err(capture_io("open final snapshot directory", &error)),
    };
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
    // The record is exactly the lowercase digest of the manifest bytes as
    // they are now; anything else is a digest mismatch, not a snapshot.
    let verified = manifest::digest(&manifest_bytes)?;
    let Ok(recorded_text) = std::str::from_utf8(&recorded_bytes) else {
        return Err(SnapshotError::ManifestDigestMismatch);
    };
    if recorded_text != verified.as_str() {
        return Err(SnapshotError::ManifestDigestMismatch);
    }
    let stored = serde_json::from_slice::<StoredManifest>(&manifest_bytes)
        .map_err(|_| SnapshotError::CapturedManifestMalformed)?;
    if stored.schema_version != 1 || stored.snapshot_id != *snapshot_id {
        return Err(SnapshotError::CapturedManifestMalformed);
    }
    if stored.entries.len() > MAX_SNAPSHOT_FILES {
        return Err(SnapshotError::TooLarge);
    }
    let mut total: u64 = 0;
    for entry in &stored.entries {
        if entry.captured_bytes > MAX_SNAPSHOT_FILE_BYTES {
            return Err(SnapshotError::TooLarge);
        }
        total = total
            .checked_add(entry.captured_bytes)
            .ok_or(SnapshotError::TooLarge)?;
        if total > MAX_SNAPSHOT_TOTAL_BYTES {
            return Err(SnapshotError::TooLarge);
        }
    }
    let unique = {
        let mut seen = HashSet::with_capacity(stored.entries.len());
        stored
            .entries
            .iter()
            .all(|entry| seen.insert(entry.path.as_str()))
    };
    if !unique {
        return Err(SnapshotError::CapturedManifestMalformed);
    }
    let entries = stored
        .entries
        .into_iter()
        .map(|entry| CapturedEntry {
            path: entry.path,
            mode: entry.mode,
            content_sha256: entry.content_sha256,
            captured_bytes: entry.captured_bytes,
            source: match entry.source {
                StoredSource::GitBlob { object_id } => SnapshotContentSource::GitBlob { object_id },
                StoredSource::WorkingTree {} => SnapshotContentSource::WorkingTree,
            },
        })
        .collect();
    Ok(CapturedSnapshot {
        snapshot_id: stored.snapshot_id,
        repository_id: stored.repository_id,
        plan_manifest_sha256: stored.plan_manifest_sha256,
        captured_manifest_sha256: verified,
        entries,
    })
}
