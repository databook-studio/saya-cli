//! Windows reset and privatize for an interrupted materialization target:
//! the target must be a real directory that is not a reparse point, every
//! entry except git's regular-file `.git` pointer is removed through the
//! bounded, never-link-following removal, and a fresh listing confirms
//! nothing else remains. Windows has no POSIX ownership or mode, so
//! `privatize` verifies shape and reparse status only.

use std::{ffi::OsString, fs, path::Path};

use super::{MAX_SNAPSHOT_FILES, SnapshotError, capture_gc_windows::remove_tree_with_limits};
use crate::workspace::windows_identity;

/// A published snapshot may hold up to `MAX_SNAPSHOT_FILES` files, so the
/// top-level listing and the re-verify accept that many entries (plus
/// `.git` slack). Removal walks at most 256 components deep.
const MAX_RESET_ENTRIES: usize = MAX_SNAPSHOT_FILES + 8;
const MAX_RESET_DEPTH: usize = 256;

pub(super) fn reset(target_root: &Path) -> Result<(), SnapshotError> {
    verify_target(target_root)?;
    let names = list_entries(target_root)?;
    let mut failures = 0usize;
    for name in &names {
        if remove_tree_with_limits(target_root, name, MAX_RESET_ENTRIES, MAX_RESET_DEPTH).is_err() {
            failures += 1;
        }
    }
    if failures > 0 {
        return Err(SnapshotError::MaterializeCleanupFailed);
    }
    verify_only_git(target_root)
}

pub(super) fn privatize(target_root: &Path) -> Result<(), SnapshotError> {
    // Git creates the worktree directory with the process defaults; Windows
    // has no mode to force. The target must still be a real directory and
    // not a link, which is what the reset and materializer rely on.
    verify_target(target_root)
}

fn verify_target(target_root: &Path) -> Result<(), SnapshotError> {
    let metadata =
        fs::symlink_metadata(target_root).map_err(|_| SnapshotError::MaterializeTargetInvalid)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(SnapshotError::MaterializeTargetInvalid);
    }
    if windows_identity::path_is_reparse_point(target_root)
        .map_err(|_| SnapshotError::MaterializeTargetInvalid)?
    {
        return Err(SnapshotError::MaterializeTargetInvalid);
    }
    Ok(())
}

/// The full bounded listing: `.git` must be a regular file, everything else
/// is collected for removal. Exhausting the bound is invalid.
fn list_entries(target_root: &Path) -> Result<Vec<OsString>, SnapshotError> {
    let mut names = Vec::new();
    for entry in fs::read_dir(target_root).map_err(|_| SnapshotError::MaterializeTargetInvalid)? {
        if names.len() == MAX_RESET_ENTRIES {
            return Err(SnapshotError::MaterializeTargetInvalid);
        }
        let entry = entry.map_err(|_| SnapshotError::MaterializeTargetInvalid)?;
        let name = entry.file_name();
        if name.to_string_lossy().eq_ignore_ascii_case(".git") {
            require_git_pointer(&entry.path())?;
            continue;
        }
        names.push(name);
    }
    Ok(names)
}

fn require_git_pointer(path: &Path) -> Result<(), SnapshotError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| SnapshotError::MaterializeTargetInvalid)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(SnapshotError::MaterializeTargetInvalid);
    }
    if windows_identity::path_is_reparse_point(path)
        .map_err(|_| SnapshotError::MaterializeTargetInvalid)?
    {
        return Err(SnapshotError::MaterializeTargetInvalid);
    }
    Ok(())
}

/// A fresh stream re-lists the directory; anything other than `.git` still
/// present means cleanup failed, closing any readdir skip.
fn verify_only_git(target_root: &Path) -> Result<(), SnapshotError> {
    for (entries, entry) in fs::read_dir(target_root)
        .map_err(|_| SnapshotError::MaterializeCleanupFailed)?
        .enumerate()
    {
        if entries == MAX_RESET_ENTRIES {
            return Err(SnapshotError::MaterializeCleanupFailed);
        }
        let entry = entry.map_err(|_| SnapshotError::MaterializeCleanupFailed)?;
        if !entry
            .file_name()
            .to_string_lossy()
            .eq_ignore_ascii_case(".git")
        {
            return Err(SnapshotError::MaterializeCleanupFailed);
        }
    }
    Ok(())
}
