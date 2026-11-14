//! The one change computation every surface reads.
//!
//! An in-place job has no inventory baseline: its changes are exactly the
//! paths its own tools first touched, compared against the before-image the
//! store kept. A managed job keeps its checkout delta. Both answer the same
//! `RootChange` shape, so readiness, checkpoints, rewind, review and status
//! cannot disagree about what the job changed.

use super::{CodingError, RootChange, checkout_changes};
use saya_agent::CancellationToken;
use saya_harness::{
    HarnessError,
    coding::{
        git::IgnoredPaths,
        integration::DeltaKind,
        local::before_image::{BeforeImageStore, BeforeState},
        snapshot::SnapshotArtifacts,
        worktree::VerifiedCheckout,
    },
    workspace::{MAX_IO_BYTES, Workspace},
};
use saya_types::Sha256Digest;
use serde_json::{Value, json};
use std::path::Path;

/// The job's own changed paths: an in-place root compares the before-image
/// store against the live folder; a managed checkout keeps its delta.
pub(crate) fn job_changes(
    artifacts: &SnapshotArtifacts,
    records: &Path,
    verified: &VerifiedCheckout,
    ignored: &IgnoredPaths,
    cancellation: &CancellationToken,
) -> Result<Vec<RootChange>, CodingError> {
    if !verified.base().is_in_place() {
        return checkout_changes(artifacts, verified, ignored, cancellation);
    }
    Ok(live(verified, records)?.changes)
}

/// An in-place root's live changes and whether its before-image store reached
/// a bound. One store read and one folder comparison, shared by every surface
/// that states what the job recorded.
pub(crate) struct LiveChanges {
    pub(crate) changes: Vec<RootChange>,
    pub(crate) overflowed: bool,
}

pub(crate) fn live(
    verified: &VerifiedCheckout,
    records: &Path,
) -> Result<LiveChanges, CodingError> {
    let store = BeforeImageStore::open(records).map_err(CodingError::owner)?;
    let workspace =
        Workspace::open(Path::new(verified.path().as_str())).map_err(CodingError::owner)?;
    let changes = in_place_tool_changes(&store, &workspace)?;
    Ok(LiveChanges {
        changes,
        overflowed: store.overflowed(),
    })
}

/// Compares each recorded first touch to the file as it stands now. An
/// unchanged path is dropped; a recorded path that is now missing is a
/// deletion; a path first seen absent that is now present is a creation; any
/// other present path is a modification, because an unrecorded prior state
/// proves nothing.
pub(crate) fn in_place_tool_changes(
    store: &BeforeImageStore,
    workspace: &Workspace,
) -> Result<Vec<RootChange>, CodingError> {
    let mut changes = Vec::new();
    for entry in store.entries().map_err(CodingError::owner)? {
        match workspace.read(&entry.path, MAX_IO_BYTES as u64) {
            Ok(file) => {
                let digest = Sha256Digest::parse(&file.digest).map_err(CodingError::owner)?;
                let executable = workspace
                    .is_executable(&entry.path)
                    .map_err(CodingError::owner)?;
                let unchanged = matches!(
                    &entry.state,
                    BeforeState::File {
                        sha256,
                        executable: before,
                        ..
                    } if *sha256 == digest && *before == executable
                );
                if !unchanged {
                    // A path the job first saw absent is its own creation; any
                    // other recorded path is a modification, because an
                    // unrecorded prior state proves nothing.
                    let kind = if matches!(entry.state, BeforeState::Absent) {
                        DeltaKind::Added
                    } else {
                        DeltaKind::Modified
                    };
                    changes.push(RootChange {
                        path: entry.path,
                        kind,
                        content_sha256: Some(digest),
                        bytes: file.size,
                    });
                }
            }
            Err(HarnessError::NotFound { .. }) => {
                // An absent path the job recorded as absent is unchanged.
                if !matches!(entry.state, BeforeState::Absent) {
                    changes.push(RootChange {
                        path: entry.path,
                        kind: DeltaKind::Deleted,
                        content_sha256: None,
                        bytes: 0,
                    });
                }
            }
            // A path the shell replaced with something this workspace refuses
            // to read is still a change; the review binds its size and mtime.
            Err(HarnessError::BoundsExceeded { found, .. }) => {
                changes.push(unreadable(entry.path, found));
            }
            Err(HarnessError::SymlinkRefused { .. } | HarnessError::NotRegularFile { .. }) => {
                changes.push(unreadable(entry.path, 0));
            }
            Err(error) => return Err(CodingError::owner(error)),
        }
    }
    Ok(changes)
}

/// A changed path the workspace cannot read: it is present, but no digest is
/// available for it.
fn unreadable(path: String, bytes: u64) -> RootChange {
    RootChange {
        path,
        kind: DeltaKind::Modified,
        content_sha256: None,
        bytes,
    }
}

/// What the job recorded about its own changes: the changed files, the bytes
/// they hold now, and whether a before-image bound was reached.
pub(crate) fn recorded_facts(changes: &[RootChange], overflowed: bool) -> Value {
    json!({
        "files": changes.len(),
        "bytes": changes.iter().fold(0u64, |total, change| {
            total.saturating_add(change.bytes)
        }),
        "overflowed": overflowed,
    })
}
