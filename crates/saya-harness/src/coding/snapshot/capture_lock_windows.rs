//! Windows advisory liveness locks for staged captures: the same protocol as
//! the unix `flock` version, expressed with `File::lock`/`try_lock`. A stage
//! holds an exclusive lock on `lock` for its whole life; the sweep removes a
//! candidate only when it can take that lock without blocking. The root-level
//! `.lock` serializes staging creation against the sweep.

use std::{
    fs::{self, File, OpenOptions},
    io,
    os::windows::fs::OpenOptionsExt as _,
    path::Path,
};

use super::{SnapshotError, capture_dest::capture_io};
use crate::workspace::{contain::FILE_FLAG_OPEN_REPARSE_POINT, windows_identity};

/// The lock file's name inside a staging directory.
pub(super) const LOCK_NAME: &str = "lock";

/// The root-level coordination file's name inside the artifacts root.
pub(super) const ROOT_LOCK_NAME: &str = ".lock";

fn open_lock(path: &Path, create: bool) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(create)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    // A link at the coordination or stage lock name is never the lock.
    if windows_identity::is_reparse_point(&file)? {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "lock name is a reparse point",
        ));
    }
    Ok(file)
}

/// Creates or opens the root coordination file so every later lock holder
/// resolves to the same file. Called once from `SnapshotArtifacts::open`.
pub(super) fn ensure_root_lock(root: &Path) -> Result<(), SnapshotError> {
    match open_lock(&root.join(ROOT_LOCK_NAME), true) {
        Ok(lock) => drop(lock),
        Err(error) => return Err(capture_io("open the artifacts root lock", &error)),
    }
    Ok(())
}

/// The exclusive root-level coordination lock. Each holder opens its own
/// handle of `.lock` and takes a blocking exclusive lock on it; dropping the
/// handle releases it.
pub(super) struct RootLock(File);

impl RootLock {
    pub(super) fn acquire(root: &Path) -> Result<Self, SnapshotError> {
        let file = open_lock(&root.join(ROOT_LOCK_NAME), false)
            .map_err(|error| capture_io("open the artifacts root lock", &error))?;
        file.lock()
            .map_err(|error| capture_io("lock the artifacts root", &error))?;
        Ok(Self(file))
    }
}

impl Drop for RootLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

/// Creates the stage's lock file exclusively inside its staging directory and
/// returns the handle holding an exclusive lock; the lock lives until the
/// stage is committed or dropped, which is what the sweep probes.
pub(super) fn acquire_stage_lock(stage: &Path) -> Result<File, SnapshotError> {
    let path = stage.join(LOCK_NAME);
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&path)
        .map_err(|error| capture_io("create staged capture lock", &error))?;
    if let Err(error) = lock.lock() {
        drop(lock);
        let _ = fs::remove_file(&path);
        return Err(capture_io("lock the staged capture", &error));
    }
    Ok(lock)
}

/// What one candidate staging directory's lock tells a sweep.
pub(super) enum StageLock {
    Free(File),
    Held,
    Absent,
}

/// Probes one staging directory's lock without blocking: a held lock never
/// removes, and any unexplainable lock state fails toward keeping the stage.
pub(super) fn probe_stage_lock(stage: &Path) -> StageLock {
    let lock = match open_lock(&stage.join(LOCK_NAME), false) {
        Ok(lock) => lock,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return StageLock::Absent,
        Err(_) => return StageLock::Held,
    };
    match lock.try_lock() {
        Ok(()) => StageLock::Free(lock),
        Err(_) => StageLock::Held,
    }
}
