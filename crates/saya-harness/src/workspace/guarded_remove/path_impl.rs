//! The non-unix body of the guarded removal: the component-by-path walk
//! with the canonicalise-then-prefix check, the verified no-follow open, the
//! bounded whole read, the digest compare — then the by-path removal.

use std::{
    fs,
    io::{self, Read as _},
};

use super::super::commit_gate::hold;
use super::super::contain::MAX_IO_BYTES;
use crate::workspace::contain::Workspace;
use crate::{HarnessError, io_error};

impl Workspace {
    /// The non-unix removal: the same admission the guarded replace runs —
    /// the scan, the verified open, the bounded whole read, the digest
    /// compare — then the by-path removal of the final component.
    pub(super) fn remove_guarded_by_path(
        &self,
        rel: &str,
        expected_sha256: &str,
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
            Err(error) => return Err(io_error("remove workspace file", &path, error)),
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
        fs::remove_file(&path).map_err(|error| io_error("remove workspace file", &path, error))
    }
}
