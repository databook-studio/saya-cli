//! Putting a checkout back to its baseline: a managed job's captured snapshot,
//! or an in-place job's recorded before-images.
//!
//! Both restore only what the job's stated write scope admits, report what it
//! does not, and leave a path a concurrent editor changed (`foreign`) as it
//! stands. A path whose before-state was deliberately not recorded is named in
//! `not_restorable` rather than guessed at.

use super::apply::{remove_current, write_back};
use super::{RestoreReport, RestoredPath};
use crate::commands::coding::CodingError;
use saya_harness::coding::integration::AssignedPaths;
use saya_harness::coding::local::before_image::{BeforeImageStore, BeforeState};
use saya_harness::coding::snapshot::{CapturedSnapshot, SnapshotArtifacts, SnapshotError};
use saya_harness::workspace::Workspace;
use std::path::Path;

/// Restores every baseline entry the job's write scope admits into
/// `checkout_root`, and removes every path the checkout added after the
/// baseline that the scope admits. Nothing is written unless the whole restore
/// fits one checkpoint's bounds; a baseline entry the scope does not admit is
/// reported and never touched, and so is a path a concurrent editor changed
/// (`foreign`).
pub(crate) fn restore_baseline(
    artifacts: &SnapshotArtifacts,
    baseline: &CapturedSnapshot,
    checkout_root: &Path,
    write_scope: &AssignedPaths,
    current_paths: &[String],
    foreign: &[String],
) -> Result<RestoreReport, CodingError> {
    let mut wanted = Vec::new();
    let mut out_of_scope = Vec::new();
    let mut total: u64 = 0;
    for entry in baseline.entries() {
        let path = entry.path().as_str();
        if !write_scope.contains(path) {
            out_of_scope.push(path.to_owned());
            continue;
        }
        if entry.captured_bytes() > super::MAX_REWIND_FILE_BYTES
            || wanted.len() >= super::MAX_REWIND_FILES_PER_CHECKPOINT
        {
            return Err(CodingError::RewindBaselineTooLarge);
        }
        total = total.saturating_add(entry.captured_bytes());
        if total > super::MAX_REWIND_CHECKPOINT_BYTES {
            return Err(CodingError::RewindBaselineTooLarge);
        }
        wanted.push(entry);
    }
    let workspace = Workspace::open(checkout_root).map_err(CodingError::owner)?;
    let mut report = RestoreReport {
        out_of_scope,
        ..RestoreReport::default()
    };
    for entry in wanted {
        let path = entry.path().as_str();
        if foreign.iter().any(|changed| changed == path) {
            report.foreign.push(path.to_owned());
            continue;
        }
        let object = artifacts
            .root()
            .join(baseline.snapshot_id().as_str())
            .join("objects")
            .join(entry.content_sha256().as_str());
        let bytes = super::super::context::read(&object, super::MAX_REWIND_FILE_BYTES as usize)?;
        if saya_types::sha256_hex(&bytes) != entry.content_sha256().as_str() {
            return Err(CodingError::owner(SnapshotError::ObjectDigestMismatch));
        }
        write_back(&workspace, path, &bytes)?;
        report.changes.push(RestoredPath {
            path: path.to_owned(),
            sha256: Some(entry.content_sha256().as_str().to_owned()),
            size: bytes.len() as u64,
        });
        report.restored += 1;
    }
    for path in current_paths {
        if !write_scope.contains(path)
            || baseline
                .entries()
                .iter()
                .any(|entry| entry.path().as_str() == path)
        {
            continue;
        }
        if foreign.iter().any(|changed| changed == path) {
            report.foreign.push(path.clone());
            continue;
        }
        remove_current(&workspace, path)?;
        report.changes.push(RestoredPath {
            path: path.clone(),
            sha256: None,
            size: 0,
        });
        report.removed += 1;
    }
    Ok(report)
}

/// Restores every path the job's own tools first touched to the before-image
/// the store kept: a recorded file gets its bytes back, a path first seen
/// absent is removed, and a deliberately unrecorded path is named in
/// `not_restorable` and left as it stands. Shell-made changes are not in the
/// store, so they are never touched — this restore is the job's own edits.
pub(crate) fn restore_before_images(
    store: &BeforeImageStore,
    checkout_root: &Path,
    write_scope: &AssignedPaths,
    foreign: &[String],
) -> Result<RestoreReport, CodingError> {
    let workspace = Workspace::open(checkout_root).map_err(CodingError::owner)?;
    let mut report = RestoreReport::default();
    for entry in store.entries().map_err(CodingError::owner)? {
        let path = entry.path.clone();
        if !write_scope.contains(&path) {
            report.out_of_scope.push(path);
            continue;
        }
        if foreign.iter().any(|changed| changed == &path) {
            report.foreign.push(path);
            continue;
        }
        match &entry.state {
            BeforeState::File { .. } => {
                match store.read_image(&entry).map_err(CodingError::owner)? {
                    Some(bytes) => {
                        write_back(&workspace, &path, &bytes)?;
                        report.changes.push(RestoredPath {
                            path,
                            sha256: Some(saya_types::sha256_hex(&bytes)),
                            size: bytes.len() as u64,
                        });
                        report.restored += 1;
                    }
                    None => report.not_restorable.push(path),
                }
            }
            BeforeState::Absent => {
                remove_current(&workspace, &path)?;
                report.changes.push(RestoredPath {
                    path,
                    sha256: None,
                    size: 0,
                });
                report.removed += 1;
            }
            BeforeState::Unrecorded(_) => report.not_restorable.push(path),
        }
    }
    Ok(report)
}
