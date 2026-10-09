//! The non-unix body of the range patch: the component-by-path walk with
//! the canonicalise-then-prefix check, the verified no-follow open, the
//! bounded whole read, and the temp+rename publication — the same
//! operations the path-based write runs.

use super::super::commit_gate::hold;
use super::super::contain::{file_identity, path_identity, replace_workspace_file};
use super::PATCH_MAX_FILE_BYTES;
use crate::workspace::contain::Workspace;
use crate::{HarnessError, io_error};

impl Workspace {
    pub(super) fn patch_range_by_path(
        &self,
        rel: &str,
        range: std::ops::Range<u64>,
        expected_len: u64,
        expected_digest: Option<&str>,
        replacement: &[u8],
    ) -> Result<(), HarnessError> {
        use std::io::Read as _;

        // The gate is held from before the fresh read to after the publish,
        // so no other Saya patch writer can interleave its own window.
        let _gate = hold();
        let path = self.target(rel, false)?;
        let pre = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(HarnessError::NotFound {
                    path: rel.to_string(),
                });
            }
            Err(error) => return Err(crate::io_error("patch workspace file", &path, error)),
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
        if pre.len() > PATCH_MAX_FILE_BYTES {
            return Err(HarnessError::BoundsExceeded {
                path: rel.to_string(),
                found: pre.len(),
                max: PATCH_MAX_FILE_BYTES,
            });
        }
        let file = self.open_verified(&path, &pre, rel)?;
        let mut current = Vec::new();
        (&file)
            .take(PATCH_MAX_FILE_BYTES.saturating_add(1))
            .read_to_end(&mut current)
            .map_err(|error| crate::io_error("read workspace file", &path, error))?;
        Self::digest_checked(rel, expected_digest, &current)?;
        let patched = Self::splice(rel, range, expected_len, replacement, &current)?;
        let parent = path.parent().unwrap_or(self.root()).to_path_buf();
        let (temp_path, mut temp) = self.create_temp(&parent, rel)?;
        use std::io::Write as _;
        temp.write_all(&patched)
            .map_err(|error| crate::io_error("write workspace temp", &temp_path, error))?;
        temp.sync_all()
            .map_err(|error| crate::io_error("sync workspace temp", &temp_path, error))?;
        let written = file_identity(&temp)
            .map_err(|error| crate::io_error("stat workspace temp", &temp_path, error))?;
        drop(temp);
        replace_workspace_file(&temp_path, &path)?;
        let final_meta = std::fs::symlink_metadata(&path)
            .map_err(|error| crate::io_error("verify written workspace file", &path, error))?;
        let committed = path_identity(&path)
            .map_err(|error| crate::io_error("verify written workspace file", &path, error))?;
        if committed != written || !final_meta.is_file() {
            return Err(HarnessError::IdentityChanged {
                path: rel.to_string(),
            });
        }
        Ok(())
    }
}
