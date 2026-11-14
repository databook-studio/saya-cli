//! The non-unix bodies of the guarded whole-file operations: the
//! component-by-path walk with the canonicalise-then-prefix check, the
//! verified no-follow open, the bounded whole read, and the temp+rename
//! publication — the same operations the path-based write and range patch
//! run.

use std::{
    fs,
    io::{self, Read as _, Write as _},
    path::Path,
};

use super::super::commit_gate::hold;
use super::super::contain::{MAX_IO_BYTES, file_identity, path_identity, replace_workspace_file};
use crate::workspace::contain::Workspace;
use crate::{HarnessError, io_error};

impl Workspace {
    /// The non-unix create: walk with parent creation allowed (the walk
    /// refuses symlinks itself), then a no-follow stat — an existing entry
    /// of any kind refuses, an absent one publishes.
    pub(super) fn create_new_by_path(&self, rel: &str, bytes: &[u8]) -> Result<(), HarnessError> {
        let _gate = hold();
        let path = self.target(rel, true)?;
        match fs::symlink_metadata(&path) {
            Ok(_) => Err(HarnessError::AlreadyExists {
                path: rel.to_string(),
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.publish_bytes(&path, rel, bytes)
            }
            Err(error) => Err(io_error("scan workspace path", &path, error)),
        }
    }

    /// The non-unix replace: the same admission the path-based patch runs —
    /// the scan, the verified open, the bounded whole read, the digest
    /// compare — then the same temp+rename publication.
    pub(super) fn write_guarded_by_path(
        &self,
        rel: &str,
        expected_sha256: &str,
        bytes: &[u8],
    ) -> Result<(), HarnessError> {
        let _gate = hold();
        let path = self.target(rel, false)?;
        let pre = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(HarnessError::NotFound {
                    path: rel.to_string(),
                });
            }
            Err(error) => return Err(io_error("write workspace file", &path, error)),
        };
        if pre.file_type().is_symlink() {
            return Err(HarnessError::SymlinkRefused {
                path: rel.to_string(),
            });
        }
        if !pre.is_file() {
            return Err(HarnessError::NotRegularFile {
                path: rel.to_string(),
            });
        }
        if pre.len() > MAX_IO_BYTES as u64 {
            return Err(HarnessError::BoundsExceeded {
                path: rel.to_string(),
                found: pre.len(),
                max: MAX_IO_BYTES as u64,
            });
        }
        let file = self.open_verified(&path, &pre, rel)?;
        let mut current = Vec::new();
        (&file)
            .take(MAX_IO_BYTES as u64 + 1)
            .read_to_end(&mut current)
            .map_err(|error| io_error("read workspace file", &path, error))?;
        Self::digest_checked(rel, Some(expected_sha256), &current)?;
        self.publish_bytes(&path, rel, bytes)
    }

    /// The non-unix atomic publication: temp at 0600 in the validated
    /// parent, fsync, rename over the target, and a post-rename identity
    /// verify so a swap after the commit is reported, not pretended away.
    fn publish_bytes(&self, path: &Path, rel: &str, bytes: &[u8]) -> Result<(), HarnessError> {
        let parent = path.parent().unwrap_or(self.root()).to_path_buf();
        let (temp_path, mut temp) = self.create_temp(&parent, rel)?;
        temp.write_all(bytes)
            .map_err(|error| io_error("write workspace temp", &temp_path, error))?;
        temp.sync_all()
            .map_err(|error| io_error("sync workspace temp", &temp_path, error))?;
        let written = file_identity(&temp)
            .map_err(|error| io_error("stat workspace temp", &temp_path, error))?;
        drop(temp);
        replace_workspace_file(&temp_path, path)?;
        let final_meta = fs::symlink_metadata(path)
            .map_err(|error| io_error("verify written workspace file", path, error))?;
        let committed = path_identity(path)
            .map_err(|error| io_error("verify written workspace file", path, error))?;
        if committed != written || !final_meta.is_file() {
            return Err(HarnessError::IdentityChanged {
                path: rel.to_string(),
            });
        }
        Ok(())
    }
}
