//! Putting one job's working checkout back to a checkpoint.
//!
//! Every write goes through the workspace's own contained operations, so a
//! restored path is validated the same way the job's tools validate one: no
//! symlink is followed, no path escapes the checkout, and a file that changed
//! since it was read is refused rather than overwritten. A path the job's
//! stated write scope does not admit is never touched at all, and is reported
//! back so the operator learns the checkpoint held one.

use super::{CapturedPath, RestoreReport, RestoredPath, RewindStore};
use crate::commands::coding::CodingError;
use saya_harness::coding::integration::AssignedPaths;
use saya_harness::coding::local::before_image::{BeforeImageEntry, BeforeImageStore, BeforeState};
use saya_harness::workspace::Workspace;
use std::collections::HashMap;
use std::path::Path;

/// Restores the checkout at `checkout_root` to checkpoint `turn`.
///
/// The in-place side of a restore: the job's before-image store (absent for a
/// managed checkout) and the paths a concurrent editor changed.
#[derive(Clone, Copy)]
pub(crate) struct Before<'a> {
    pub(crate) images: Option<&'a BeforeImageStore>,
    pub(crate) foreign: &'a [String],
}

/// `current_paths` is every path the checkout currently changes against the
/// job's baseline — the job's own delta. A path there that the checkpoint does
/// not hold was created after it, so matching the checkpoint means removing the
/// file; an in-place job instead restores that path's before-image, and removes
/// it only when the job first saw it absent. `write_scope` is the job's stated
/// scope, parsed: nothing outside it is read, written, or removed. `only`, when
/// stated, restricts both passes to those exact paths; every other changed path
/// is left as it stands. `foreign` names paths a concurrent editor changed:
/// they are reported and never touched.
pub(crate) fn restore(
    store: &RewindStore,
    checkout_root: &Path,
    write_scope: &AssignedPaths,
    turn: u32,
    current_paths: &[String],
    only: Option<&[String]>,
    before: Before<'_>,
) -> Result<RestoreReport, CodingError> {
    let checkpoint = store.images(turn)?;
    let before_entries = before_entries(before.images)?;
    let workspace = Workspace::open(checkout_root).map_err(CodingError::owner)?;
    let mut report = RestoreReport::default();
    let mut recorded: Vec<&str> = Vec::with_capacity(checkpoint.len());
    for path in &checkpoint {
        if !wanted(only, &path.path) {
            continue;
        }
        if !write_scope.contains(&path.path) {
            report.out_of_scope.push(path.path.clone());
            continue;
        }
        recorded.push(&path.path);
        if before.foreign.iter().any(|changed| changed == &path.path) {
            report.foreign.push(path.path.clone());
            continue;
        }
        let (sha256, size) = match &path.bytes {
            Some(bytes) => {
                write_back(&workspace, &path.path, bytes)?;
                (Some(saya_types::sha256_hex(bytes)), bytes.len() as u64)
            }
            None => {
                remove_current(&workspace, &path.path)?;
                (None, 0)
            }
        };
        report.changes.push(RestoredPath {
            path: path.path.clone(),
            sha256,
            size,
        });
        report.restored += 1;
    }
    // A path the checkout changes today but the checkpoint never recorded did
    // not exist at that checkpoint. An in-place job restores the bytes its
    // first touch recorded; only a path first seen absent is removed.
    for path in current_paths {
        if !wanted(only, path) || recorded.contains(&path.as_str()) || !write_scope.contains(path) {
            continue;
        }
        if before.foreign.iter().any(|changed| changed == path) {
            report.foreign.push(path.clone());
            continue;
        }
        if let Some(store) = before.images {
            match before_entries.get(path) {
                Some(entry) if matches!(entry.state, BeforeState::File { .. }) => {
                    let bytes = store
                        .read_image(entry)
                        .map_err(CodingError::owner)?
                        .ok_or(CodingError::RewindCaptureTooLarge)?;
                    write_back(&workspace, path, &bytes)?;
                    report.changes.push(RestoredPath {
                        path: path.clone(),
                        sha256: Some(saya_types::sha256_hex(&bytes)),
                        size: bytes.len() as u64,
                    });
                    report.restored += 1;
                    continue;
                }
                Some(entry) if matches!(entry.state, BeforeState::Absent) => {
                    remove_current(&workspace, path)?;
                    report.changes.push(RestoredPath {
                        path: path.clone(),
                        sha256: None,
                        size: 0,
                    });
                    report.removed += 1;
                    continue;
                }
                // An unrecorded first touch has no bytes to restore, and no
                // absence to prove: it is reported, never guessed at.
                _ => {
                    report.not_restorable.push(path.clone());
                    continue;
                }
            }
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

/// The before-image index, read once for a restore's second pass.
fn before_entries(
    store: Option<&BeforeImageStore>,
) -> Result<HashMap<String, BeforeImageEntry>, CodingError> {
    let Some(store) = store else {
        return Ok(HashMap::new());
    };
    Ok(store
        .entries()
        .map_err(CodingError::owner)?
        .into_iter()
        .map(|entry| (entry.path.clone(), entry))
        .collect())
}

/// Whether `path` is in the caller's restriction; no restriction admits every
/// path.
pub(super) fn wanted(only: Option<&[String]>, path: &str) -> bool {
    only.is_none_or(|paths| paths.iter().any(|wanted| wanted == path))
}

/// Writes the checkpointed bytes over the file as it stands, creating it when
/// the job removed it after the checkpoint.
pub(super) fn write_back(
    workspace: &Workspace,
    path: &str,
    bytes: &[u8],
) -> Result<(), CodingError> {
    match workspace.read(path, super::MAX_REWIND_FILE_BYTES) {
        Ok(current) => workspace
            .write_guarded(path, &current.digest, bytes)
            .map_err(CodingError::owner),
        Err(saya_harness::HarnessError::NotFound { .. }) => workspace
            .create_new(path, bytes)
            .map_err(CodingError::owner),
        Err(error) => Err(CodingError::owner(error)),
    }
}

/// Removes the file when it is there. An already-absent path is already at the
/// checkpoint's state, which is not a failure.
pub(super) fn remove_current(workspace: &Workspace, path: &str) -> Result<(), CodingError> {
    match workspace.read(path, super::MAX_REWIND_FILE_BYTES) {
        Ok(current) => workspace
            .remove_guarded(path, &current.digest)
            .map_err(CodingError::owner),
        Err(saya_harness::HarnessError::NotFound { .. }) => Ok(()),
        Err(error) => Err(CodingError::owner(error)),
    }
}

/// The checkout's current changed paths and the state each holds, read for a
/// capture. A path over the per-file bound is not captured at all: a partial
/// image would restore a file to something it never was.
pub(crate) fn read_paths(
    workspace: &Workspace,
    paths: &[String],
) -> Result<Vec<CapturedPath>, CodingError> {
    let mut captured = Vec::with_capacity(paths.len());
    for path in paths {
        match workspace.read(path, super::MAX_REWIND_FILE_BYTES) {
            Ok(file) if !file.truncated => captured.push(CapturedPath {
                path: path.clone(),
                bytes: Some(file.bytes),
            }),
            Ok(_) => return Err(CodingError::RewindCaptureTooLarge),
            Err(saya_harness::HarnessError::NotFound { .. }) => captured.push(CapturedPath {
                path: path.clone(),
                bytes: None,
            }),
            Err(error) => return Err(CodingError::owner(error)),
        }
    }
    Ok(captured)
}
