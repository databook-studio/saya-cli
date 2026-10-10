//! The private snapshot-artifacts destination: a root the trusted
//! composition root supplies, one staging directory per capture, and the
//! stale-staging sweep. Supported on macOS and Linux and fail-closed
//! elsewhere. The root is created or verified as one entry of its pinned
//! parent (0700, exclusive, never chmod-ed when it already exists), staging
//! is created exclusively under a pinned root descriptor, and each stage
//! holds an advisory lock the sweep must acquire before it may remove
//! anything; `begin` and the sweep additionally serialize on a root-level
//! lock so a sweep can never see a stage between its creation and its lock.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use saya_types::SnapshotId;

use super::{SnapshotError, capture_staging::StagedSnapshot};

#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::{
    ffi::{OsStr, OsString},
    fs, io,
    os::fd::AsRawFd,
    process,
    sync::Arc,
};

#[cfg(any(target_os = "macos", target_os = "linux"))]
use super::capture_dest_private::{mkdir_private_at, open_or_create_private_root, privatize_dir};

#[cfg(any(target_os = "macos", target_os = "linux"))]
use crate::workspace::{
    contain::nanos,
    fd::{open_dir_at, open_root_fd, stat_at},
};

/// An artifact id must stay a single safe path component; this mirrors the
/// snapshot-id shape guarantee and re-checks it at the artifacts boundary,
/// refusing rather than sanitizing.
const MAX_ARTIFACT_ID_BYTES: usize = 128;

#[cfg(any(target_os = "macos", target_os = "linux"))]
const MAX_STAGING_ATTEMPTS: usize = 8;

pub struct SnapshotArtifacts {
    root: PathBuf,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    root_dir: Arc<fs::File>,
}

impl SnapshotArtifacts {
    /// Opens (creating, 0700, if absent) the private artifacts root the
    /// trusted composition root supplies. On unsupported targets this
    /// refuses.
    pub fn open(root: &Path) -> Result<Self, SnapshotError> {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            Self::open_unix(root)
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            let _ = root;
            Err(SnapshotError::ArtifactRootRefused)
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn open_unix(root: &Path) -> Result<Self, SnapshotError> {
        // The root is created or verified as one entry of its pinned
        // parent: the parent must already exist and is never chmod-ed, an
        // absent root name is created exclusively at 0700, and an existing
        // entry is accepted only when it already is an owned-private
        // directory — never repaired into one.
        let name = root.file_name().ok_or(SnapshotError::ArtifactRootRefused)?;
        let parent = root
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .ok_or(SnapshotError::ArtifactRootRefused)?;
        let parent_dir = match open_root_fd(parent.as_os_str()) {
            Ok(fd) => fs::File::from(fd),
            Err(error) if error.raw_os_error() == Some(libc::ELOOP) => {
                return Err(SnapshotError::ArtifactRootRefused);
            }
            Err(error) => return Err(capture_io("open artifacts root parent", &error)),
        };
        let file = open_or_create_private_root(&parent_dir, name)?;
        // The root coordination file every `begin` and sweep serializes on
        // exists from here on.
        super::capture_lock::ensure_root_lock(&file)?;
        Ok(Self {
            root: root.to_path_buf(),
            root_dir: Arc::new(file),
        })
    }

    /// The artifacts root as supplied by the composition root. Every
    /// operation acts through the pinned descriptor, not this path.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The pinned artifacts root descriptor for the verified reader.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub(super) fn root_dir(&self) -> &fs::File {
        &self.root_dir
    }

    /// Opens a staging directory for one capture of `snapshot_id`: the id
    /// is re-validated as a safe component, any existing entry under the id
    /// refuses, and a unique `.staging-<id>-<pid>-<nanos>` directory (0700,
    /// exclusive, with its `objects/` child and its held `lock` file) is
    /// created for the write.
    pub fn begin(&self, snapshot_id: &SnapshotId) -> Result<StagedSnapshot, SnapshotError> {
        validate_artifact_id(snapshot_id)?;
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            self.begin_unix(snapshot_id)
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            Err(SnapshotError::ArtifactRootRefused)
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn begin_unix(&self, snapshot_id: &SnapshotId) -> Result<StagedSnapshot, SnapshotError> {
        // Any existing final entry — directory, file, or link — refuses; the
        // no-replace rename at commit remains the atomic arbiter for races
        // after this.
        let id_name = OsStr::new(snapshot_id.as_str());
        match stat_at(self.root_dir.as_raw_fd(), id_name) {
            Ok(_) => return Err(SnapshotError::SnapshotExists),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(capture_io("scan final snapshot directory", &error)),
        }
        // The root lock covers everything from the staging `mkdirat` to the
        // stage's own lock being held: a concurrent sweep — this process or
        // another — cannot even start scanning in that window, so it can
        // never observe a staging directory that exists but does not yet
        // hold its lock. The lock is dropped as soon as the stage lock is
        // held; the stage's own flock keeps the stage safe from then on.
        let _root_lock = super::capture_lock::RootLock::acquire(&self.root_dir)?;
        let (staging_name, staging_dir) = self.create_staging_dir(snapshot_id)?;
        mkdir_private_at(staging_dir.as_raw_fd(), OsStr::new("objects"))
            .map_err(|error| capture_io("create staged objects directory", &error))?;
        let objects_dir = fs::File::from(
            open_dir_at(staging_dir.as_raw_fd(), OsStr::new("objects"))
                .map_err(|error| capture_io("open staged objects directory", &error))?,
        );
        privatize_dir(&objects_dir, "staged objects directory")?;
        // The stage's advisory lock lives inside its staging directory and
        // is held on this descriptor for the stage's whole life.
        let lock_file = super::capture_lock::acquire_stage_lock(&staging_dir)?;
        drop(_root_lock);
        // Durable: `objects/` and `lock` exist before any object temp.
        staging_dir
            .sync_all()
            .map_err(|error| capture_io("sync staging directory", &error))?;
        Ok(StagedSnapshot::new(
            Arc::clone(&self.root_dir),
            staging_dir,
            objects_dir,
            lock_file,
            staging_name,
            snapshot_id.clone(),
        ))
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn create_staging_dir(
        &self,
        snapshot_id: &SnapshotId,
    ) -> Result<(OsString, fs::File), SnapshotError> {
        let base = format!(
            ".staging-{}-{}-{}",
            snapshot_id.as_str(),
            process::id(),
            nanos()
        );
        for attempt in 0..MAX_STAGING_ATTEMPTS {
            let name = if attempt == 0 {
                OsString::from(base.clone())
            } else {
                OsString::from(format!("{base}-{}", nanos()))
            };
            match mkdir_private_at(self.root_dir.as_raw_fd(), &name) {
                Ok(()) => {
                    // The reopened descriptor is compared against the entry
                    // creation made and is then the one descriptor every
                    // content operation goes through.
                    let dir = super::capture_dest_private::reopen_created_private_dir(
                        &self.root_dir,
                        &name,
                        "staging directory",
                    )?;
                    privatize_dir(&dir, "staging directory")?;
                    return Ok((name, dir));
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(capture_io("create staging directory", &error)),
            }
        }
        Err(SnapshotError::CaptureIo {
            description: "name a unique staging directory".to_string(),
            source: None,
        })
    }

    /// True only when the final directory for `snapshot_id` holds a
    /// verifiable completion record whose digest matches its manifest.
    /// Missing or malformed state is `Ok(false)`, never an error.
    pub fn is_complete(&self, snapshot_id: &SnapshotId) -> Result<bool, SnapshotError> {
        validate_artifact_id(snapshot_id)?;
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            super::capture_complete::is_complete(&self.root_dir, snapshot_id)
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            Err(SnapshotError::ArtifactRootRefused)
        }
    }

    /// Removes `.staging-*` siblings whose recorded nanoseconds are older
    /// than `older_than` and whose advisory lock the sweep can acquire.
    /// Stale staging never blocks `begin` and never presents as complete;
    /// entries that cannot be parsed, locked, or removed stay as inert
    /// garbage for a later sweep.
    pub fn sweep_stale_staging(&self, older_than: Duration) -> Result<usize, SnapshotError> {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            super::capture_dest_gc::sweep_stale(&self.root_dir, older_than)
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            let _ = older_than;
            Err(SnapshotError::ArtifactRootRefused)
        }
    }
}

pub(super) fn validate_artifact_id(snapshot_id: &SnapshotId) -> Result<(), SnapshotError> {
    let value = snapshot_id.as_str();
    if value.is_empty()
        || value.len() > MAX_ARTIFACT_ID_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        // The charset excludes '.', '..', '/', '\', NUL, and non-ASCII, so
        // no refused shape can escape the artifacts root.
        return Err(SnapshotError::ArtifactPathInvalid);
    }
    Ok(())
}

/// Maps an io failure to a bounded, path-free description: a fixed context
/// phrase, the error kind, and the raw os code. Never the raw message,
/// which can embed paths.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) fn capture_io(context: &'static str, error: &io::Error) -> SnapshotError {
    let mut description = format!("{context} ({:?})", error.kind());
    if let Some(code) = error.raw_os_error() {
        description.push_str(&format!(", os error {code}"));
    }
    SnapshotError::CaptureIo {
        description,
        source: None,
    }
}
