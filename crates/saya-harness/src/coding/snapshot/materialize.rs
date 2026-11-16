//! The materialization orchestrator: replays one verified published
//! snapshot into a private, empty target. The target path is app-owned:
//! the caller supplies a path and a symlink at that path is refused.
//! Each stored object is opened no-follow under the pinned artifacts
//! root and re-hashed while it streams into the target in 64 KiB
//! blocks, so a corrupted store can only produce a typed refusal —
//! never materialized bytes. On any failure after the target opens the
//! target is rolled back to empty, unless cleanup itself fails and
//! `MaterializeCleanupFailed` is returned: the target must then be
//! discarded. A concurrent writer into the target during
//! materialization is outside the contract — the target must be
//! private and unshared. The receipt binds content digests only; it is
//! not an ownership proof (the binding layer binds
//! `(snapshot_id, captured_manifest_sha256)`), and the plan manifest's
//! canonical bytes are persisted by the binding layer, not by the
//! artifacts.

use saya_agent::CancellationToken;
use saya_types::SnapshotId;
use std::path::Path;

use super::{
    SnapshotArtifacts, SnapshotError,
    materialize_copy::{copy_entry, validate_symlink_set},
    materialize_types::MaterializedSnapshot,
};

#[cfg(any(target_os = "macos", target_os = "linux"))]
use super::materialize_target::TargetWriter;
#[cfg(windows)]
use super::materialize_target_windows::TargetWriter;
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
use super::{CapturedSnapshot, materialize_types::MaterializedEntry};

#[cfg(any(target_os = "macos", target_os = "linux", windows))]
use crate::workspace::Workspace;

#[cfg(any(target_os = "macos", target_os = "linux", windows))]
use std::fs;

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
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    {
        materialize_supported(artifacts, snapshot_id, target_root, cancellation)
    }
    #[cfg(all(not(any(target_os = "macos", target_os = "linux")), not(windows)))]
    {
        let _ = (artifacts, snapshot_id, target_root, cancellation);
        Err(SnapshotError::ArtifactRootRefused)
    }
}

/// The supported-target body: strict read-back, guarded target path,
/// per-entry verified copy, rollback on every error, created-directory
/// fsync on success where the platform has directory fsync.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
fn materialize_supported(
    artifacts: &SnapshotArtifacts,
    snapshot_id: &SnapshotId,
    target_root: &Path,
    cancellation: &CancellationToken,
) -> Result<MaterializedSnapshot, SnapshotError> {
    let target_metadata = fs::symlink_metadata(target_root).map_err(|error| {
        super::capture_dest::capture_io("verify materialization target", &error)
    })?;
    if target_metadata.file_type().is_symlink() || !target_metadata.is_dir() {
        return Err(SnapshotError::MaterializeTargetInvalid);
    }
    let workspace = Workspace::open(target_root).map_err(|error| match error {
        crate::HarnessError::NotRegularFile { .. } => SnapshotError::MaterializeTargetInvalid,
        other => SnapshotError::from_workspace_error(other),
    })?;
    let captured = artifacts.read_snapshot(snapshot_id)?;
    let mut writer = TargetWriter::open(&workspace)?;
    let result = materialize_entries(&captured, artifacts, &mut writer, cancellation);
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
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
fn materialize_entries(
    captured: &CapturedSnapshot,
    artifacts: &SnapshotArtifacts,
    writer: &mut TargetWriter<'_>,
    cancellation: &CancellationToken,
) -> Result<MaterializedSnapshot, SnapshotError> {
    validate_symlink_set(captured, artifacts, cancellation)?;
    let mut entries = Vec::with_capacity(captured.entries().len());
    let mut total: u64 = 0;
    for entry in captured.entries() {
        if cancellation.is_cancelled() {
            return Err(SnapshotError::Cancelled);
        }
        let bytes = copy_entry(
            artifacts,
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
