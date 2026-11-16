//! The Windows snapshot-artifacts destination: the same private-root and
//! staging contract as the unix implementation, with reparse-point refusal
//! standing in for POSIX ownership and modes, and path operations standing in
//! for the pinned directory descriptors.

use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    process,
};

use saya_types::SnapshotId;

use super::super::{
    capture_lock_windows::{RootLock, acquire_stage_lock, ensure_root_lock},
    capture_staging::StagedSnapshot,
};
use super::{SnapshotArtifacts, SnapshotError, capture_io};
use crate::workspace::{contain::nanos, windows_identity};

/// Staging names get a fresh suffix on a collision, exactly as on unix.
const MAX_STAGING_ATTEMPTS: usize = 8;

pub(super) fn open(root: &Path) -> Result<SnapshotArtifacts, SnapshotError> {
    // The root is created or verified as one entry of its pinned parent: the
    // parent must already exist, an absent root name is created, and an
    // existing entry is accepted only when it already is a real directory.
    let name = root.file_name().ok_or(SnapshotError::ArtifactRootRefused)?;
    let parent = root
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or(SnapshotError::ArtifactRootRefused)?;
    let parent_metadata = fs::symlink_metadata(parent)
        .map_err(|error| capture_io("open artifacts root parent", &error))?;
    if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
        return Err(SnapshotError::ArtifactRootRefused);
    }
    let parent = fs::canonicalize(parent)
        .map_err(|error| capture_io("open artifacts root parent", &error))?;
    if windows_identity::path_is_reparse_point(&parent)
        .map_err(|error| capture_io("open artifacts root parent", &error))?
    {
        return Err(SnapshotError::ArtifactRootRefused);
    }
    let candidate = parent.join(name);
    match fs::symlink_metadata(&candidate) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(SnapshotError::ArtifactRootRefused);
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(&candidate)
                .map_err(|error| capture_io("create artifacts root", &error))?;
        }
        Err(error) => return Err(capture_io("open artifacts root", &error)),
    }
    if windows_identity::path_is_reparse_point(&candidate)
        .map_err(|error| capture_io("open artifacts root", &error))?
    {
        return Err(SnapshotError::ArtifactRootRefused);
    }
    let root = simplify(
        fs::canonicalize(&candidate).map_err(|error| capture_io("open artifacts root", &error))?,
    );
    ensure_root_lock(&root)?;
    Ok(SnapshotArtifacts { root })
}

pub(super) fn begin(
    root: &Path,
    snapshot_id: &SnapshotId,
) -> Result<StagedSnapshot, SnapshotError> {
    // Any existing final entry — directory, file, or link — refuses; the
    // no-replace rename at commit remains the atomic arbiter for races.
    match fs::symlink_metadata(root.join(snapshot_id.as_str())) {
        Ok(_) => return Err(SnapshotError::SnapshotExists),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(capture_io("scan final snapshot directory", &error)),
    }
    // The root lock covers the staging creation through the stage lock, so a
    // concurrent sweep can never see a stage that exists without its lock.
    let root_lock = RootLock::acquire(root)?;
    let (staging_name, staging_path) = create_staging_dir(root, snapshot_id)?;
    let objects_path = staging_path.join("objects");
    fs::create_dir(&objects_path)
        .map_err(|error| capture_io("create staged objects directory", &error))?;
    if windows_identity::path_is_reparse_point(&objects_path)
        .map_err(|error| capture_io("open staged objects directory", &error))?
    {
        return Err(SnapshotError::CaptureIo {
            description: "staged objects directory is a reparse point".to_string(),
            source: None,
        });
    }
    let lock_file = acquire_stage_lock(&staging_path)?;
    drop(root_lock);
    StagedSnapshot::new_windows(
        root.to_path_buf(),
        staging_path,
        objects_path,
        lock_file,
        staging_name,
        snapshot_id.clone(),
    )
}

fn create_staging_dir(
    root: &Path,
    snapshot_id: &SnapshotId,
) -> Result<(OsString, PathBuf), SnapshotError> {
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
        let path = root.join(&name);
        match fs::create_dir(&path) {
            Ok(()) => {
                if windows_identity::path_is_reparse_point(&path)
                    .map_err(|error| capture_io("create staging directory", &error))?
                {
                    return Err(SnapshotError::CaptureIo {
                        description: "staging directory is a reparse point".to_string(),
                        source: None,
                    });
                }
                return Ok((name, path));
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

/// Strip the `\\?\` verbatim prefix `canonicalize` returns so stored and
/// compared paths keep the ordinary spelling.
fn simplify(path: PathBuf) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path;
    };
    let Some(rest) = text.strip_prefix(r"\\?\") else {
        return path;
    };
    match rest.strip_prefix(r"UNC\") {
        Some(unc) => PathBuf::from(format!(r"\\{unc}")),
        None => PathBuf::from(rest),
    }
}
