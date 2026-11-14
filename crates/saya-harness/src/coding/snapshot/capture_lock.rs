//! Advisory liveness locks for staged captures. Each stage creates a
//! `lock` file inside its own staging directory at creation and holds an
//! exclusive `flock` on it for the stage's whole life; the stale-staging
//! sweep removes a candidate only when it can acquire that lock without
//! blocking, so a live capture is never swept. A killed capture's lock is
//! released by the kernel at process death, and a staging directory without
//! a lock file is either a crashed leftover or a stage mid-removal, after
//! its drop unlinked the lock before the staging directory itself is
//! removed — for either, age alone decides.
//!
//! On top of the per-stage lock, stage creation and sweeping are
//! serialized through the root-level `.lock` file: `begin` holds an
//! exclusive `flock` on a fresh open of it from before the staging
//! `mkdirat` until the stage lock is held, and the sweep holds it for its
//! whole scan-and-remove pass. Every operation opens its own file
//! description, so the exclusion works across threads and processes; a
//! sweep can therefore never observe a staging directory that is between
//! `mkdirat` and its stage lock acquisition. A staging directory without a
//! lock file is then either a crashed leftover or a stage mid-removal,
//! removable by age in either case.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::{ffi::OsStr, fs, io, os::fd::AsRawFd};

use super::{SnapshotError, capture_dest::capture_io};
use crate::workspace::fd::{open_file_at, unlink_at};

/// The lock file's name inside a staging directory.
pub(super) const LOCK_NAME: &str = "lock";

/// The root-level coordination file's name inside the artifacts root.
pub(super) const ROOT_LOCK_NAME: &str = ".lock";

/// Creates or opens the root coordination file (0600, never following a
/// link at its name) so every later flock holder — this process or
/// another — resolves to the same inode. Called once from
/// `SnapshotArtifacts::open`; the descriptor is dropped, the flock is
/// taken per operation instead.
pub(super) fn ensure_root_lock(root_dir: &fs::File) -> Result<(), SnapshotError> {
    match open_file_at(
        root_dir.as_raw_fd(),
        OsStr::new(ROOT_LOCK_NAME),
        libc::O_WRONLY | libc::O_CREAT,
        0o600,
    ) {
        Ok(lock) => drop(lock),
        Err(error) if error.raw_os_error() == Some(libc::ELOOP) => {
            // A link at the coordination name is not the root's own file.
            return Err(SnapshotError::ArtifactRootRefused);
        }
        Err(error) => return Err(capture_io("open the artifacts root lock", &error)),
    }
    Ok(())
}

/// The exclusive root-level coordination lock. Each holder opens its own
/// file description of `.lock` and takes a blocking `flock(LOCK_EX)` on
/// it; dropping the handle closes the descriptor and releases the flock.
/// It is held across exactly one serialized section — staging-directory
/// creation through stage-lock acquisition in `begin`, and the sweep's
/// whole scan-and-remove pass — and never across anything slower.
pub(super) struct RootLock(fs::File);

impl RootLock {
    /// Fresh-opened `.lock` plus a blocking exclusive `flock`: the fresh
    /// open is what makes the exclusion hold across threads and processes
    /// sharing the same root.
    pub(super) fn acquire(root_dir: &fs::File) -> Result<Self, SnapshotError> {
        let file = match open_file_at(root_dir.as_raw_fd(), OsStr::new(ROOT_LOCK_NAME), 0, 0) {
            Ok(file) => file,
            Err(error) if error.raw_os_error() == Some(libc::ELOOP) => {
                return Err(SnapshotError::ArtifactRootRefused);
            }
            Err(error) => return Err(capture_io("open the artifacts root lock", &error)),
        };
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            let error = io::Error::last_os_error();
            return Err(capture_io("lock the artifacts root", &error));
        }
        Ok(Self(file))
    }
}

impl Drop for RootLock {
    fn drop(&mut self) {
        // Releasing explicitly before the close: the flock ends with the
        // section, not whenever the descriptor is reaped.
        let _ = unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
    }
}

/// Creates the stage's lock file exclusively inside its staging directory
/// and returns the descriptor holding `flock(LOCK_EX)`; the flock lives
/// until the stage is committed or dropped, which is what the sweep probes.
pub(super) fn acquire_stage_lock(stage_dir: &fs::File) -> Result<fs::File, SnapshotError> {
    let lock = open_file_at(
        stage_dir.as_raw_fd(),
        OsStr::new(LOCK_NAME),
        libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
        0o600,
    )
    .map_err(|error| capture_io("create staged capture lock", &error))?;
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
        let error = io::Error::last_os_error();
        drop(lock);
        let _ = unlink_at(stage_dir.as_raw_fd(), OsStr::new(LOCK_NAME));
        return Err(capture_io("lock the staged capture", &error));
    }
    Ok(lock)
}

/// What one candidate staging directory's lock tells a sweep. `Free` hands
/// back the descriptor the sweep's own exclusive lock is held on and must
/// be kept alive until the candidate is removed; `Held` means a live
/// capture holds the lock, or the lock cannot be verified at all (a link,
/// an unreadable file) — either way the stage is kept, never removed;
/// `Absent` means the capture died before creating its lock or the stage is
/// mid-removal after its lock was unlinked, and only the age decides.
pub(super) enum StageLock {
    Free(fs::File),
    Held,
    Absent,
}

/// Probes one opened staging directory's lock without blocking: a held
/// `flock(LOCK_EX | LOCK_NB)` never removes, and any unexplainable lock
/// state fails toward keeping the stage.
pub(super) fn probe_stage_lock(stage_dir: &fs::File) -> StageLock {
    let lock = match open_file_at(stage_dir.as_raw_fd(), OsStr::new(LOCK_NAME), 0, 0) {
        Ok(lock) => lock,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return StageLock::Absent,
        Err(_) => return StageLock::Held,
    };
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        StageLock::Free(lock)
    } else {
        StageLock::Held
    }
}
