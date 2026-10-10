//! Reset of an interrupted materialization target: returns a Creating
//! checkout's target to `.git`-pointer-only by removing every entry the
//! interrupted materialization left behind. Supported on macOS and Linux
//! only; every other target refuses.

use std::path::Path;

use super::SnapshotError;

/// Returns the target at `target_root` to `.git`-pointer-only: every
/// entry except git's regular-file `.git` pointer is removed, bounded
/// and never following links. Only for a checkout still in the Creating
/// state — never for a checkout that has been handed out.
pub(crate) fn reset_materialization_target(target_root: &Path) -> Result<(), SnapshotError> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        reset_unix(target_root)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = target_root;
        Err(SnapshotError::ArtifactRootRefused)
    }
}

/// Ensures the managed materialization target is a directory owned by the
/// current effective user and sets it to mode 0700: `git worktree add`
/// creates it with the process umask (typically 0755), while
/// materialization and reset require a process-owned 0700 directory. The
/// pre-chmod check is ownership only — git may have left group/other bits
/// set — and the chmod runs on the held `O_DIRECTORY|O_NOFOLLOW`
/// descriptor (fchmod, no path re-resolution) before a full
/// owned-private re-verification.
pub(crate) fn privatize_materialization_target(target_root: &Path) -> Result<(), SnapshotError> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        privatize_unix(target_root)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = target_root;
        Err(SnapshotError::ArtifactRootRefused)
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::{
    ffi::{OsStr, OsString},
    fs,
    os::{fd::AsRawFd, unix::ffi::OsStrExt},
};

#[cfg(any(target_os = "macos", target_os = "linux"))]
use super::{
    MAX_SNAPSHOT_FILES, capture_dest_gc::remove_tree_with_limits,
    capture_dest_private::owned_private,
};

#[cfg(any(target_os = "macos", target_os = "linux"))]
use crate::workspace::fd::{DirStream, open_root_fd, stat_at};

/// Reset's own bounds: a published snapshot may hold up to
/// `MAX_SNAPSHOT_FILES` files, so the top-level listing and the
/// re-verify accept that many entries (plus `.git` slack) — the old
/// 4096-entry bound could strand a Creating retry on a valid large
/// snapshot. The removal walks at most 256 components deep: a 512-byte
/// snapshot path has at most 256 components.
#[cfg(any(target_os = "macos", target_os = "linux"))]
const MAX_RESET_ENTRIES: usize = MAX_SNAPSHOT_FILES + 8;
#[cfg(any(target_os = "macos", target_os = "linux"))]
const MAX_RESET_DEPTH: usize = 256;

/// The unix body: shape and ownership verified through a pinned
/// `O_DIRECTORY|O_NOFOLLOW` descriptor, then three bounded phases: the
/// whole directory is listed into a `Vec` before any removal (never
/// unlinking while a directory stream is still being read, which POSIX
/// leaves unspecified), the collected names are removed with failures
/// counted, and a fresh re-list confirms nothing but `.git` remains,
/// closing any readdir skip.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn reset_unix(target_root: &Path) -> Result<(), SnapshotError> {
    let target_metadata =
        fs::symlink_metadata(target_root).map_err(|_| SnapshotError::MaterializeTargetInvalid)?;
    if target_metadata.file_type().is_symlink() || !target_metadata.is_dir() {
        return Err(SnapshotError::MaterializeTargetInvalid);
    }
    let root = fs::File::from(
        open_root_fd(target_root.as_os_str())
            .map_err(|_| SnapshotError::MaterializeTargetInvalid)?,
    );
    let metadata = root
        .metadata()
        .map_err(|_| SnapshotError::MaterializeTargetInvalid)?;
    if !metadata.is_dir() || !owned_private(&metadata) {
        return Err(SnapshotError::MaterializeTargetInvalid);
    }
    let names = list_entries(&root)?;
    let mut failures = 0usize;
    for name in &names {
        if remove_tree_with_limits(&root, name, MAX_RESET_ENTRIES, MAX_RESET_DEPTH).is_err() {
            failures += 1;
        }
    }
    if failures > 0 {
        return Err(SnapshotError::MaterializeCleanupFailed);
    }
    verify_only_git(&root)
}

/// The unix body: shape verified through symlink metadata and a pinned
/// `O_DIRECTORY|O_NOFOLLOW` descriptor, ownership checked before the
/// chmod (git's umask mode is accepted), the mode forced to 0700 on the
/// held descriptor, and a full owned-private re-verification after it.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn privatize_unix(target_root: &Path) -> Result<(), SnapshotError> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
    let target_metadata =
        fs::symlink_metadata(target_root).map_err(|_| SnapshotError::MaterializeTargetInvalid)?;
    if target_metadata.file_type().is_symlink() || !target_metadata.is_dir() {
        return Err(SnapshotError::MaterializeTargetInvalid);
    }
    let root = fs::File::from(
        open_root_fd(target_root.as_os_str())
            .map_err(|_| SnapshotError::MaterializeTargetInvalid)?,
    );
    let metadata = root
        .metadata()
        .map_err(|_| SnapshotError::MaterializeTargetInvalid)?;
    if !metadata.is_dir() || metadata.uid() != unsafe { libc::geteuid() } {
        return Err(SnapshotError::MaterializeTargetInvalid);
    }
    root.set_permissions(fs::Permissions::from_mode(0o700))
        .map_err(|_| SnapshotError::MaterializeTargetInvalid)?;
    let metadata = root
        .metadata()
        .map_err(|_| SnapshotError::MaterializeTargetInvalid)?;
    if !owned_private(&metadata) {
        return Err(SnapshotError::MaterializeTargetInvalid);
    }
    Ok(())
}

/// Phase 1: the full bounded listing, `.` and `..` skipped, `.git`
/// validated as a regular file exactly as before. Exhausting the entry
/// bound without reaching the end of the stream is invalid.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn list_entries(root: &fs::File) -> Result<Vec<OsString>, SnapshotError> {
    let mut stream = DirStream::open_at(root.as_raw_fd(), OsStr::new("."))
        .map_err(|_| SnapshotError::MaterializeTargetInvalid)?;
    let mut names = Vec::new();
    let mut complete = false;
    for _ in 0..MAX_RESET_ENTRIES {
        let Some((name, _)) = stream
            .next()
            .map_err(|_| SnapshotError::MaterializeTargetInvalid)?
        else {
            complete = true;
            break;
        };
        if name.as_bytes() == b"." || name.as_bytes() == b".." {
            continue;
        }
        if name.as_bytes() == b".git" {
            let pointer = stat_at(root.as_raw_fd(), &name)
                .map_err(|_| SnapshotError::MaterializeTargetInvalid)?;
            if !pointer.is_file() {
                return Err(SnapshotError::MaterializeTargetInvalid);
            }
            continue;
        }
        names.push(name);
    }
    if !complete {
        return Err(SnapshotError::MaterializeTargetInvalid);
    }
    Ok(names)
}

/// Phase 3: a fresh stream re-lists the directory; anything other than
/// `.git` still present means cleanup failed, closing any readdir skip.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn verify_only_git(root: &fs::File) -> Result<(), SnapshotError> {
    let mut stream = DirStream::open_at(root.as_raw_fd(), OsStr::new("."))
        .map_err(|_| SnapshotError::MaterializeCleanupFailed)?;
    for _ in 0..MAX_RESET_ENTRIES {
        let Some((name, _)) = stream
            .next()
            .map_err(|_| SnapshotError::MaterializeCleanupFailed)?
        else {
            return Ok(());
        };
        if name.as_bytes() == b"." || name.as_bytes() == b".." || name.as_bytes() == b".git" {
            continue;
        }
        return Err(SnapshotError::MaterializeCleanupFailed);
    }
    Err(SnapshotError::MaterializeCleanupFailed)
}
