//! The materialization orchestrator: replays one verified published
//! snapshot into a private, empty target. The target path is app-owned:
//! the caller supplies a path and a symlink at that path is refused.
//! Each stored object is opened no-follow under the pinned artifacts
//! root and re-hashed while it streams into the target in 64 KiB
//! chunks, so a corrupted store can only produce a typed refusal —
//! never materialized bytes. On any failure after the target opens the
//! target is rolled back to empty, unless cleanup itself fails and
//! `MaterializeCleanupFailed` is returned: the target must then be
//! discarded. A concurrent writer into the target during
//! materialization is outside the contract — the target must be
//! private and unshared. The receipt binds content digests only; it is
//! not an ownership proof (W05 binds
//! `(snapshot_id, captured_manifest_sha256)`), and the plan manifest's
//! canonical bytes are persisted by the binding layer, not by the
//! artifacts.

use saya_agent::CancellationToken;
use saya_types::SnapshotId;
use std::path::Path;

#[cfg(any(target_os = "macos", target_os = "linux"))]
use saya_types::Sha256Digest;

use super::{SnapshotArtifacts, SnapshotError, materialize_types::MaterializedSnapshot};
#[cfg(any(target_os = "macos", target_os = "linux"))]
use crate::workspace::Workspace;

#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::{
    ffi::OsStr,
    fs,
    io::{Read as _, Write as _},
    os::fd::AsRawFd,
};

#[cfg(any(target_os = "macos", target_os = "linux"))]
use sha2::{Digest as _, Sha256};

#[cfg(any(target_os = "macos", target_os = "linux"))]
use super::{
    CapturedEntry, CapturedSnapshot, capture_dest::capture_io, manifest,
    materialize_target::TargetWriter, materialize_types::MaterializedEntry,
    types::MAX_SNAPSHOT_TOTAL_BYTES,
};

#[cfg(any(target_os = "macos", target_os = "linux"))]
use crate::workspace::fd::{open_dir_at, open_file_at};

/// Materializes the published snapshot `snapshot_id` from `artifacts`
/// into the empty private directory at `target_root`, which must be
/// app-owned: a symlink at that path is refused. Entries are written in
/// manifest order, the target is synced, and the receipt is returned.
/// On any failure after the target opens — including cancellation —
/// the target is rolled back to empty; if cleanup itself fails,
/// `MaterializeCleanupFailed` is returned and the target must be
/// discarded. Refuses on unsupported targets.
pub fn materialize_snapshot(
    artifacts: &SnapshotArtifacts,
    snapshot_id: &SnapshotId,
    target_root: &Path,
    cancellation: &CancellationToken,
) -> Result<MaterializedSnapshot, SnapshotError> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        materialize_unix(artifacts, snapshot_id, target_root, cancellation)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (artifacts, snapshot_id, target_root, cancellation);
        Err(SnapshotError::ArtifactRootRefused)
    }
}

/// The copy chunk size: the same 64 KiB streaming granularity the
/// anchored capture reader uses.
#[cfg(any(target_os = "macos", target_os = "linux"))]
const CHUNK_BYTES: usize = 64 * 1024;

/// The unix body: strict read-back, guarded target path, per-entry
/// verified copy, rollback on every error, created-directory fsync on
/// success.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn materialize_unix(
    artifacts: &SnapshotArtifacts,
    snapshot_id: &SnapshotId,
    target_root: &Path,
    cancellation: &CancellationToken,
) -> Result<MaterializedSnapshot, SnapshotError> {
    let target_metadata = fs::symlink_metadata(target_root)
        .map_err(|error| capture_io("verify materialization target", &error))?;
    if target_metadata.file_type().is_symlink() || !target_metadata.is_dir() {
        return Err(SnapshotError::MaterializeTargetInvalid);
    }
    let workspace = Workspace::open(target_root).map_err(|error| match error {
        crate::HarnessError::NotRegularFile { .. } => SnapshotError::MaterializeTargetInvalid,
        other => SnapshotError::from_workspace_error(other),
    })?;
    let captured = artifacts.read_snapshot(snapshot_id)?;
    let root_dir = artifacts.root_dir();
    let mut writer = TargetWriter::open(&workspace)?;
    let result = materialize_entries(&captured, root_dir, &mut writer, cancellation);
    let result = match result {
        Ok(receipt) => writer.sync_all().map(|()| receipt),
        Err(error) => Err(error),
    };
    match result {
        Ok(receipt) => Ok(receipt),
        Err(error) => match writer.rollback() {
            Err(rollback @ SnapshotError::MaterializeCleanupFailed) => Err(rollback),
            _ => Err(error),
        },
    }
}

/// Copies every captured entry in manifest order, re-verifying each
/// object's bytes against its declared digest and size, and assembles
/// the receipt.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn materialize_entries(
    captured: &CapturedSnapshot,
    root_dir: &fs::File,
    writer: &mut TargetWriter<'_>,
    cancellation: &CancellationToken,
) -> Result<MaterializedSnapshot, SnapshotError> {
    let mut entries = Vec::with_capacity(captured.entries().len());
    let mut total: u64 = 0;
    for entry in captured.entries() {
        if cancellation.is_cancelled() {
            return Err(SnapshotError::Cancelled);
        }
        let bytes = copy_entry(
            root_dir,
            captured.snapshot_id(),
            entry,
            writer,
            &mut total,
            cancellation,
        )?;
        entries.push(MaterializedEntry {
            path: entry.path().clone(),
            mode: entry.mode(),
            content_sha256: entry.content_sha256().clone(),
            bytes,
        });
    }
    Ok(MaterializedSnapshot {
        snapshot_id: captured.snapshot_id().clone(),
        plan_manifest_sha256: captured.plan_manifest_sha256().clone(),
        captured_manifest_sha256: captured.captured_manifest_sha256().clone(),
        entries,
    })
}

/// Opens the stored object for one entry no-follow — the id directory,
/// then `objects/`, then the content-addressed name — and refuses
/// non-regular entries on the opened descriptor.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn open_object(
    root_dir: &fs::File,
    snapshot_id: &SnapshotId,
    content_sha256: &Sha256Digest,
) -> Result<fs::File, SnapshotError> {
    let id_dir = open_dir_at(root_dir.as_raw_fd(), OsStr::new(snapshot_id.as_str()))
        .map_err(|error| capture_io("open stored snapshot directory", &error))
        .map(fs::File::from)?;
    let objects_dir = open_dir_at(id_dir.as_raw_fd(), OsStr::new("objects"))
        .map_err(|error| capture_io("open stored object directory", &error))
        .map(fs::File::from)?;
    let object = open_file_at(
        objects_dir.as_raw_fd(),
        OsStr::new(content_sha256.as_str()),
        libc::O_RDONLY | libc::O_NONBLOCK,
        0,
    )
    .map_err(|error| capture_io("open stored snapshot object", &error))?;
    if !object
        .metadata()
        .map_err(|error| capture_io("scan stored snapshot object", &error))?
        .is_file()
    {
        return Err(SnapshotError::ObjectDigestMismatch);
    }
    Ok(object)
}

/// Streams one stored object into its target file, refusing oversize
/// counts, oversize totals, short objects, and digest mismatches.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn copy_entry(
    root_dir: &fs::File,
    snapshot_id: &SnapshotId,
    entry: &CapturedEntry,
    writer: &mut TargetWriter<'_>,
    total: &mut u64,
    cancellation: &CancellationToken,
) -> Result<u64, SnapshotError> {
    let mut object = open_object(root_dir, snapshot_id, entry.content_sha256())?;
    let mut out = writer.create_file(entry.path(), entry.mode())?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; CHUNK_BYTES];
    let mut count: u64 = 0;
    loop {
        if cancellation.is_cancelled() {
            return Err(SnapshotError::Cancelled);
        }
        let read = object
            .read(&mut buffer)
            .map_err(|error| capture_io("read stored snapshot object", &error))?;
        if read == 0 {
            break;
        }
        count = count
            .checked_add(read as u64)
            .ok_or(SnapshotError::ObjectDigestMismatch)?;
        if count > entry.captured_bytes() {
            return Err(SnapshotError::ObjectDigestMismatch);
        }
        *total = (*total)
            .checked_add(read as u64)
            .ok_or(SnapshotError::TooLarge)?;
        if *total > MAX_SNAPSHOT_TOTAL_BYTES {
            return Err(SnapshotError::TooLarge);
        }
        out.write_all(&buffer[..read])
            .map_err(|error| capture_io("write materialized file", &error))?;
        hasher.update(&buffer[..read]);
    }
    if count != entry.captured_bytes() {
        return Err(SnapshotError::ObjectDigestMismatch);
    }
    let actual = manifest::digest_from_hasher(hasher)?;
    if actual != *entry.content_sha256() {
        return Err(SnapshotError::ObjectDigestMismatch);
    }
    writer.finish_file(out, entry.mode())?;
    Ok(count)
}
