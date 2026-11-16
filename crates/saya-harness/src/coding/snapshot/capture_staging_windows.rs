//! The Windows staged capture: the same object temps, completion record, and
//! no-replace publication as the unix descriptor version, over paths. The
//! staging directory's identity is pinned at creation and re-checked before
//! the manifest is written and again immediately before the publish rename,
//! so a swapped staging name is refused rather than adopted. The stage lock
//! is the only handle this stage keeps inside the staging directory, and it
//! is released before the publish rename: Windows refuses to rename a
//! directory that still contains an open handle (`ERROR_ACCESS_DENIED`),
//! where the unix version can rename with its directory descriptors open.
//! Windows has no directory fsync; `MOVEFILE_WRITE_THROUGH` on the publish
//! rename is the durability boundary, and each file is flushed before it is
//! renamed.

use std::{
    ffi::OsString,
    fs::{self, OpenOptions},
    io::Write as _,
    os::windows::fs::OpenOptionsExt as _,
    path::PathBuf,
};

use saya_types::{Sha256Digest, SnapshotId};

use super::super::{
    capture_complete_windows::{COMPLETION_RECORD, MANIFEST_FILE},
    capture_dest::capture_io,
    capture_lock_windows::RootLock,
    capture_object_sink::ObjectSink,
    capture_publish_windows,
    capture_staging::StagedSnapshot,
    manifest,
    types::MAX_SNAPSHOT_MANIFEST_BYTES,
};
use crate::workspace::{
    contain::{FILE_FLAG_OPEN_REPARSE_POINT, nanos},
    windows_identity,
};

/// Internal retries before a unique object temp name gives up.
const MAX_OBJECT_NAME_ATTEMPTS: usize = 8;

impl StagedSnapshot {
    /// Assembles the staged capture `begin` created: the staging directory
    /// and its `objects/` child already exist and `lock_file` holds the
    /// stage's exclusive lock. The directory's identity is pinned here, so
    /// commit can refuse a name that no longer holds this directory.
    pub(crate) fn new_windows(
        root: PathBuf,
        staging_path: PathBuf,
        objects_path: PathBuf,
        lock_file: fs::File,
        staging_name: OsString,
        snapshot_id: SnapshotId,
    ) -> Result<Self, super::super::SnapshotError> {
        let staging_identity = windows_identity::directory_identity(&staging_path)
            .map_err(|error| capture_io("pin staged snapshot directory", &error))?;
        Ok(Self {
            root,
            staging_path,
            objects_path,
            lock_file: Some(lock_file),
            staging_name,
            snapshot_id,
            remaining: super::super::types::MAX_SNAPSHOT_TOTAL_BYTES,
            next_object: 0,
            committed: false,
            staging_identity,
        })
    }
}

/// Opens one object's exclusive temp file inside the staging directory,
/// named by an internal counter; no caller path ever names an object.
pub(super) fn new_object(
    stage: &mut StagedSnapshot,
) -> Result<ObjectSink<'_>, super::super::SnapshotError> {
    let base = format!("object-{:08}", stage.next_object);
    for attempt in 0..MAX_OBJECT_NAME_ATTEMPTS {
        let name = if attempt == 0 {
            base.clone()
        } else {
            format!("{base}-{}", nanos())
        };
        let path = stage.staging_path.join(&name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&path)
        {
            Ok(file) => return Ok(ObjectSink::new(stage, file, path)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(capture_io("create staged object temp", &error)),
        }
    }
    Err(super::super::SnapshotError::CaptureIo {
        description: "name a unique staged object temp".to_string(),
        source: None,
    })
}

/// Verifies the manifest digest, writes `manifest.json`, then last the
/// `COMPLETE` record carrying that digest, flushes both files, and publishes
/// the staging directory as `<root>/<id>` with a no-replace, write-through
/// rename. Errors before the rename leave removable staging for drop.
pub(super) fn commit(
    mut stage: StagedSnapshot,
    manifest_bytes: &[u8],
    manifest_sha256: &Sha256Digest,
) -> Result<(), super::super::SnapshotError> {
    if manifest_bytes.len() > MAX_SNAPSHOT_MANIFEST_BYTES {
        return Err(super::super::SnapshotError::TooLarge);
    }
    let actual = manifest::digest(manifest_bytes)?;
    if actual != *manifest_sha256 {
        return Err(super::super::SnapshotError::ManifestDigestMismatch);
    }
    verify_staging_identity(&stage, "write staged manifest")?;
    write_new_file(&stage.staging_path, MANIFEST_FILE, manifest_bytes)?;
    // Last: the record's existence is what makes the directory apparently
    // complete, so it lands after the manifest it names.
    write_new_file(
        &stage.staging_path,
        COMPLETION_RECORD,
        manifest_sha256.as_str().as_bytes(),
    )?;
    // Windows refuses to rename a directory that still contains an open
    // handle (`ERROR_ACCESS_DENIED`, os error 5). The stage lock is the only
    // handle this stage keeps inside the staging directory — the manifest
    // and completion writers are closed when they return, and every object
    // sink has already finished — so releasing it is the whole release.
    // The root lock is held from the release through the rename: the sweep
    // only reclaims a staging directory whose lock it can take, so without
    // it a concurrent sweep could remove this now-unlocked directory in the
    // gap between the release and the rename.
    let root_lock = RootLock::acquire(&stage.root)?;
    drop(stage.lock_file.take());
    // The staging name must still hold the very directory this stage was
    // created on, immediately before the publish rename acts on the name.
    // The check runs with no handle held inside the directory, so the state
    // it verifies is the state the rename sees. A writer with artifacts-root
    // access could still swap the name between this check and the rename;
    // that is the same best-effort boundary the unix path accepts, where an
    // open directory descriptor does not pin a name either.
    verify_staging_identity(&stage, "publish staged snapshot")?;
    observe_before_publish(&stage);
    let final_path = stage.root.join(stage.snapshot_id.as_str());
    // No-replace publication: an existing final name of any kind refuses
    // instead of being replaced.
    match capture_publish_windows::rename_no_replace(&stage.staging_path, &final_path) {
        Ok(()) => {}
        Err(error)
            if matches!(
                error.raw_os_error(),
                Some(ERROR_FILE_EXISTS) | Some(ERROR_ALREADY_EXISTS)
            ) =>
        {
            return Err(super::super::SnapshotError::SnapshotExists);
        }
        Err(error) => return Err(capture_io("publish staged snapshot", &error)),
    }
    stage.committed = true;
    drop(root_lock);
    // The lock file must not publish: it is already closed, so unlinking it
    // from the renamed directory removes only the name. A failure here
    // leaves only an inert extra file, never an apparent-complete change.
    let _ = fs::remove_file(final_path.join(super::super::capture_lock_windows::LOCK_NAME));
    Ok(())
}

/// `ERROR_FILE_EXISTS` and `ERROR_ALREADY_EXISTS`: the no-replace rename
/// found the final name already taken.
const ERROR_FILE_EXISTS: i32 = 80;
const ERROR_ALREADY_EXISTS: i32 = 183;

// Test-only observation point at the publish boundary: a unit test installs
// an observer and drives a real commit, so it can assert the exact state the
// rename acts on — in particular that no handle this stage held inside the
// staging directory is still open. Compiled out of every non-test build.
#[cfg(test)]
type BeforePublishObserver = Box<dyn FnOnce(&StagedSnapshot)>;

#[cfg(test)]
thread_local! {
    static BEFORE_PUBLISH: std::cell::RefCell<Option<BeforePublishObserver>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn set_before_publish_observer(observer: BeforePublishObserver) {
    BEFORE_PUBLISH.with(|slot| *slot.borrow_mut() = Some(observer));
}

#[cfg(test)]
fn observe_before_publish(stage: &StagedSnapshot) {
    if let Some(observer) = BEFORE_PUBLISH.with(|slot| slot.borrow_mut().take()) {
        observer(stage);
    }
}

#[cfg(not(test))]
fn observe_before_publish(_stage: &StagedSnapshot) {}

fn verify_staging_identity(
    stage: &StagedSnapshot,
    context: &'static str,
) -> Result<(), super::super::SnapshotError> {
    let named = windows_identity::directory_identity(&stage.staging_path)
        .map_err(|error| capture_io(context, &error))?;
    if named == stage.staging_identity {
        Ok(())
    } else {
        Err(super::super::SnapshotError::CaptureIo {
            description: format!("keep {context} under its own name"),
            source: None,
        })
    }
}

fn write_new_file(
    dir: &std::path::Path,
    name: &str,
    bytes: &[u8],
) -> Result<(), super::super::SnapshotError> {
    let path = dir.join(name);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&path)
        .map_err(|error| capture_io("create staged file", &error))?;
    file.write_all(bytes)
        .map_err(|error| capture_io("write staged file", &error))?;
    file.sync_all()
        .map_err(|error| capture_io("sync staged file", &error))?;
    Ok(())
}

/// Drops an uncommitted stage best-effort: the lock is released first
/// because Windows refuses to remove a directory whose lock file is held,
/// then the bounded, never-link-following removal removes the staging tree.
impl Drop for StagedSnapshot {
    fn drop(&mut self) {
        let _ = self.lock_file.take();
        if !self.committed {
            let _ = super::super::capture_gc_windows::remove_tree_bounded(
                &self.root,
                &self.staging_name,
            );
        }
    }
}

#[cfg(test)]
#[path = "capture_staging_windows_tests.rs"]
mod tests;
