//! One staged capture in progress: exclusive 0600 object temps inside the
//! staging directory, and the last-write completion record followed by the
//! single no-replace rename that publishes the final directory and fsyncs
//! it into durability. Dropping an uncommitted stage removes its staging
//! directory best-effort, so cancel, error, and panic paths never leave a
//! partial snapshot — at worst inert staging garbage.

use saya_types::{Sha256Digest, SnapshotId};

use super::{SnapshotError, capture_object_sink::ObjectSink};

#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::{
    ffi::{OsStr, OsString},
    fs, io,
    os::fd::AsRawFd,
    sync::Arc,
};

#[cfg(any(target_os = "macos", target_os = "linux"))]
use super::{
    capture_dest::capture_io,
    manifest,
    types::{MAX_SNAPSHOT_MANIFEST_BYTES, MAX_SNAPSHOT_TOTAL_BYTES},
};

#[cfg(any(target_os = "macos", target_os = "linux"))]
use crate::workspace::fd::{open_file_at, unlink_at};

/// Internal retries before a unique object temp name gives up.
#[cfg(any(target_os = "macos", target_os = "linux"))]
const MAX_OBJECT_NAME_ATTEMPTS: usize = 8;
/// The completion record `commit` writes last, and the manifest file it
/// writes first; both names are internal to the staged capture.
#[cfg(any(target_os = "macos", target_os = "linux"))]
const COMPLETION_RECORD: &str = "COMPLETE";
#[cfg(any(target_os = "macos", target_os = "linux"))]
const MANIFEST_FILE: &str = "manifest.json";

/// The write side of one staged capture. Constructed only by
/// [`SnapshotArtifacts::begin`](super::SnapshotArtifacts::begin); W04d's
/// orchestrator composes `begin` → `new_object`/`finish` per entry →
/// `commit` to publish.
pub struct StagedSnapshot {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    root_dir: Arc<fs::File>,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub(super) staging_dir: fs::File,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub(super) objects_dir: fs::File,
    /// The stage's advisory lock, held (and released at drop or commit) for
    /// the stage's whole life so the stale-staging sweep can never remove a
    /// live capture. Always `Some` where staging exists; the lock does not
    /// exist elsewhere because staging is not supported there.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    lock_file: Option<fs::File>,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    staging_name: OsString,
    snapshot_id: SnapshotId,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub(super) remaining: u64,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub(super) next_object: u64,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    committed: bool,
}

impl StagedSnapshot {
    /// Assembles the staged capture `begin` created: the staging directory
    /// and its `objects/` child already exist at 0700, and `lock_file`
    /// holds the stage's exclusive advisory lock. `pub(crate)` because
    /// only the destination constructs one.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub(crate) fn new(
        root_dir: Arc<fs::File>,
        staging_dir: fs::File,
        objects_dir: fs::File,
        lock_file: fs::File,
        staging_name: OsString,
        snapshot_id: SnapshotId,
    ) -> Self {
        Self {
            root_dir,
            staging_dir,
            objects_dir,
            lock_file: Some(lock_file),
            staging_name,
            snapshot_id,
            remaining: MAX_SNAPSHOT_TOTAL_BYTES,
            next_object: 0,
            committed: false,
        }
    }

    /// The id the final directory will take once committed.
    pub fn snapshot_id(&self) -> &SnapshotId {
        &self.snapshot_id
    }

    /// Opens one object's exclusive 0600 temp file inside the staging
    /// directory, named by an internal counter — no caller path ever names
    /// an object. The sink enforces the stage's aggregate byte bound.
    pub fn new_object(&mut self) -> Result<ObjectSink<'_>, SnapshotError> {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            self.new_object_unix()
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            Err(SnapshotError::ArtifactRootRefused)
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn new_object_unix(&mut self) -> Result<ObjectSink<'_>, SnapshotError> {
        let base = format!("object-{:08}", self.next_object);
        for attempt in 0..MAX_OBJECT_NAME_ATTEMPTS {
            let name = if attempt == 0 {
                OsString::from(base.clone())
            } else {
                OsString::from(format!("{base}-{}", crate::workspace::contain::nanos()))
            };
            match open_file_at(
                self.staging_dir.as_raw_fd(),
                OsStr::new(&name),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
                0o600,
            ) {
                Ok(file) => return Ok(ObjectSink::new(self, file, name)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(capture_io("create staged object temp", &error)),
            }
        }
        Err(SnapshotError::CaptureIo {
            description: "name a unique staged object temp".to_string(),
            source: None,
        })
    }

    /// Verifies the manifest bytes against their declared digest, writes
    /// `manifest.json`, then last the `COMPLETE` record carrying that
    /// digest, fsyncs the files and directories, and atomically renames the
    /// staging directory to `<root>/<id>` with a no-replace rename. Errors
    /// before the rename leave only removable staging, removed by drop;
    /// `SnapshotExists` refuses without replacing an existing directory;
    /// `PublishedButNotDurable` means the directory was already published
    /// under the id but a post-rename durability sync failed.
    pub fn commit(
        self,
        manifest_bytes: &[u8],
        manifest_sha256: &Sha256Digest,
    ) -> Result<(), SnapshotError> {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            let mut this = self;
            this.commit_unix(manifest_bytes, manifest_sha256)
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            let _ = (self, manifest_bytes, manifest_sha256);
            Err(SnapshotError::ArtifactRootRefused)
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn commit_unix(
        &mut self,
        manifest_bytes: &[u8],
        manifest_sha256: &Sha256Digest,
    ) -> Result<(), SnapshotError> {
        if manifest_bytes.len() > MAX_SNAPSHOT_MANIFEST_BYTES {
            return Err(SnapshotError::TooLarge);
        }
        let actual = manifest::digest(manifest_bytes)?;
        if actual != *manifest_sha256 {
            return Err(SnapshotError::ManifestDigestMismatch);
        }
        self.write_private_file(MANIFEST_FILE, manifest_bytes)?;
        // Last: the record's existence is what makes the directory
        // apparently complete, so it lands after the manifest it names.
        self.write_private_file(COMPLETION_RECORD, manifest_sha256.as_str().as_bytes())?;
        self.staging_dir
            .sync_all()
            .map_err(|error| capture_io("sync staging directory", &error))?;
        self.objects_dir
            .sync_all()
            .map_err(|error| capture_io("sync staged objects directory", &error))?;
        let root_fd = self.root_dir.as_raw_fd();
        // Name-based boundary: re-establish that the staging name still
        // holds the very directory this stage's retained descriptor was
        // opened on, immediately before the publish rename acts on the
        // name. A mismatch refuses instead of renaming a stranger's entry.
        super::capture_dest_private::verify_named_identity(
            &self.root_dir,
            &self.staging_name,
            &self.staging_dir,
            "publish staged snapshot",
        )?;
        // No-replace publication: an existing final name of any kind
        // refuses instead of being replaced, so a concurrent commit of the
        // same id can never swap a live snapshot for a fresh one.
        match super::capture_publish::rename_no_replace(
            root_fd,
            &self.staging_name,
            root_fd,
            OsStr::new(self.snapshot_id.as_str()),
        ) {
            Ok(()) => {}
            Err(error)
                if error.raw_os_error() == Some(libc::EEXIST)
                    || error.raw_os_error() == Some(libc::ENOTEMPTY) =>
            {
                return Err(SnapshotError::SnapshotExists);
            }
            Err(error) => return Err(capture_io("publish staged snapshot", &error)),
        }
        self.committed = true;
        // The lock file must not publish: the renamed directory is the
        // final snapshot, so its own descriptor unlinks the lock. A failure
        // here leaves only an inert extra file, never an apparent-complete
        // change.
        let _ = unlink_at(
            self.staging_dir.as_raw_fd(),
            OsStr::new(super::capture_lock::LOCK_NAME),
        );
        // Durability: `Ok` is returned only after the renamed directory and
        // the root entry are synced. A failure here returns
        // `PublishedButNotDurable`: the directory is already published under
        // the id, so this is not a pre-publish io error and the caller must
        // not treat the snapshot as absent or retry the capture.
        self.staging_dir
            .sync_all()
            .map_err(|_| SnapshotError::PublishedButNotDurable)?;
        self.root_dir
            .sync_all()
            .map_err(|_| SnapshotError::PublishedButNotDurable)?;
        Ok(())
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn write_private_file(&mut self, name: &str, bytes: &[u8]) -> Result<(), SnapshotError> {
        use std::io::Write as _;
        let mut file = open_file_at(
            self.staging_dir.as_raw_fd(),
            OsStr::new(name),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
            0o600,
        )
        .map_err(|error| capture_io("create staged file", &error))?;
        file.write_all(bytes)
            .map_err(|error| capture_io("write staged file", &error))?;
        file.sync_all()
            .map_err(|error| capture_io("sync staged file", &error))?;
        Ok(())
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl Drop for StagedSnapshot {
    fn drop(&mut self) {
        if !self.committed {
            // Best-effort, bounded, never link-following: a failure leaves
            // inert `.staging-*` garbage for `sweep_stale_staging`. The
            // stage's own flock is still held while this runs, but the tree
            // removal unlinks the lock file first, so a sweep arriving
            // mid-drop can see no lock and finish removing the stage by
            // age; that is harmless, the stage was being removed. The name-based
            // removal first re-establishes that the staging name still
            // holds the directory this stage's descriptor was opened on; a
            // mismatch leaves the stranger's entry alone as sweepable
            // garbage.
            if super::capture_dest_private::verify_named_identity(
                &self.root_dir,
                &self.staging_name,
                &self.staging_dir,
                "remove cancelled staging",
            )
            .is_ok()
            {
                let _ =
                    super::capture_dest_gc::remove_tree_bounded(&self.root_dir, &self.staging_name);
            }
        }
        // The lock file descriptor drops last, releasing the flock.
        let _ = self.lock_file.take();
    }
}
