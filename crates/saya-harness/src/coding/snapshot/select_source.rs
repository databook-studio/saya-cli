//! The working-tree half of snapshot selection: what an untracked or modified
//! path actually is, and how it is fingerprinted for the manifest. Unix
//! anchors every step to a parent descriptor and records the real inode and
//! executable bit; Windows has neither `openat` nor an executable bit, so it
//! validates the parent through the contained component walk, scans the final
//! component without following it, and plans every regular file as `Regular`.
//! Both refuse a special file and prove a reported deletion before treating a
//! missing path as an error.

use saya_agent::CancellationToken;
use tokio::time::Instant;

#[cfg(not(unix))]
use crate::workspace::contain::path_identity;
use crate::{coding::git::GitPathStatus, workspace::Workspace};

use super::{
    manifest::SnapshotError,
    select_support::status_proves_deletion,
    symlink::{SymlinkSet, validate_symlink_target},
    types::*,
};

/// The fingerprint of one working-tree path: where its bytes come from, the
/// mode the snapshot records, its declared size, and (on unix) the identity
/// re-checked when the bytes are read.
pub(super) type SourceFacts = (
    SnapshotContentSource,
    SnapshotFileMode,
    u64,
    Option<SnapshotSourceIdentity>,
);

/// Classifies the working-tree entry at `path`. `Ok(None)` means the status
/// proves the path was deleted and it is not job content.
pub(super) fn classify_working_tree(
    workspace: &Workspace,
    path: &str,
    status: &[GitPathStatus],
    links: &mut SymlinkSet,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<Option<SourceFacts>, SnapshotError> {
    let _ = (cancellation, deadline);
    #[cfg(unix)]
    {
        let anchor = match workspace.anchor_link(path) {
            Ok(anchor) => anchor,
            Err(crate::HarnessError::NotFound { .. }) if status_proves_deletion(status, path) => {
                return Ok(None);
            }
            Err(crate::HarnessError::NotFound { .. }) => return Err(SnapshotError::unsafe_path()),
            Err(error) => return Err(SnapshotError::from_workspace_error(error)),
        };
        let stat = match anchor.stat() {
            Some(stat) => stat,
            None if status_proves_deletion(status, path) => return Ok(None),
            None => return Err(SnapshotError::unsafe_path()),
        };
        let (device, inode) = stat.identity();
        if stat.is_symlink() {
            let target = anchor
                .read_link(MAX_SYMLINK_TARGET_BYTES)
                .map_err(|error| {
                    if error.kind() == std::io::ErrorKind::InvalidData {
                        SnapshotError::unsafe_symlink(path, "the target is longer than 4096 bytes")
                    } else {
                        SnapshotError::CaptureIo {
                            description: "read the snapshot symlink".to_owned(),
                            source: Some(super::manifest::RedactedIoError::new(error)),
                        }
                    }
                })?;
            validate_symlink_target(path, &target)?;
            let size = target.len() as u64;
            links.insert(path, target);
            Ok(Some((
                SnapshotContentSource::WorkingTree,
                SnapshotFileMode::Symlink,
                size,
                Some(SnapshotSourceIdentity::new(device, inode, size, 0o120000)),
            )))
        } else if stat.is_file() {
            let executable = stat.has_exec_bits();
            Ok(Some((
                SnapshotContentSource::WorkingTree,
                if executable {
                    SnapshotFileMode::Executable
                } else {
                    SnapshotFileMode::Regular
                },
                stat.len(),
                Some(SnapshotSourceIdentity::new(
                    device,
                    inode,
                    stat.len(),
                    if executable { 0o100755 } else { 0o100644 },
                )),
            )))
        } else {
            Err(SnapshotError::UnsupportedFileType)
        }
    }
    #[cfg(not(unix))]
    {
        // The path-based walk: the parent is validated by the contained
        // component walk, and the final component is scanned without
        // following it so a link is captured as a link.
        let (parent_rel, name) = match path.rsplit_once('/') {
            Some((parent, name)) => (parent, name),
            None => ("", path),
        };
        let parent = if parent_rel.is_empty() {
            workspace.root().to_path_buf()
        } else {
            workspace
                .target(parent_rel, false)
                .map_err(SnapshotError::from_workspace_error)?
        };
        let target = parent.join(name);
        let meta = match std::fs::symlink_metadata(&target) {
            Ok(meta) => meta,
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && status_proves_deletion(status, path) =>
            {
                return Ok(None);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(SnapshotError::unsafe_path());
            }
            Err(error) => {
                return Err(SnapshotError::CaptureIo {
                    description: "scan the snapshot source".to_owned(),
                    source: Some(super::manifest::RedactedIoError::new(error)),
                });
            }
        };
        if meta.file_type().is_symlink() {
            let link = std::fs::read_link(&target).map_err(|error| SnapshotError::CaptureIo {
                description: "read the snapshot symlink".to_owned(),
                source: Some(super::manifest::RedactedIoError::new(error)),
            })?;
            // Windows spells a link target with backslashes; the snapshot
            // format is the git-style forward-slash spelling, and the
            // validator refuses backslashes.
            let link = link.to_string_lossy().replace('\\', "/");
            if link.len() > MAX_SYMLINK_TARGET_BYTES {
                return Err(SnapshotError::unsafe_symlink(
                    path,
                    "the target is longer than 4096 bytes",
                ));
            }
            validate_symlink_target(path, link.as_bytes())?;
            let size = link.len() as u64;
            links.insert(path, link.into_bytes());
            Ok(Some((
                SnapshotContentSource::WorkingTree,
                SnapshotFileMode::Symlink,
                size,
                Some(SnapshotSourceIdentity::new(0, 0, size, 0o120000)),
            )))
        } else if meta.is_file() {
            let (device, inode) =
                path_identity(&target).map_err(|error| SnapshotError::CaptureIo {
                    description: "identify the snapshot source".to_owned(),
                    source: Some(super::manifest::RedactedIoError::new(error)),
                })?;
            let size = meta.len();
            Ok(Some((
                SnapshotContentSource::WorkingTree,
                SnapshotFileMode::Regular,
                size,
                Some(SnapshotSourceIdentity::new(device, inode, size, 0o100644)),
            )))
        } else {
            Err(SnapshotError::UnsupportedFileType)
        }
    }
}
