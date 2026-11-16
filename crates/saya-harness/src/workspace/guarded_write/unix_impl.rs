//! The unix bodies of the guarded whole-file operations: the anchored walk,
//! the no-follow verified open, the bounded whole read, and the shared
//! atomic commit — the same operations the anchored range patch runs.

use std::io::Read as _;

use super::super::anchored::final_stat;
use super::super::commit_gate::hold;
use super::super::contain::MAX_IO_BYTES;
use crate::workspace::contain::Workspace;
use crate::{HarnessError, io_error};

impl Workspace {
    /// The unix create: walk with parent creation allowed, then the final
    /// component's no-follow scan — `Some` is an existing entry of any kind
    /// and refuses; `None` publishes through the shared atomic commit.
    pub(super) fn create_new_anchored(&self, rel: &str, bytes: &[u8]) -> Result<(), HarnessError> {
        let _gate = hold();
        let anchor = self.anchor(rel, true)?;
        if anchor.stat().is_some() {
            return Err(HarnessError::AlreadyExists {
                path: rel.to_string(),
            });
        }
        anchor.commit_bytes(rel, bytes)
    }

    /// The unix replace: the same admission the anchored patch runs — the
    /// stat, the verified no-follow open, the bounded whole read, the digest
    /// compare — then the shared commit carrying the caller's bytes.
    pub(super) fn write_guarded_anchored(
        &self,
        rel: &str,
        expected_sha256: &str,
        bytes: &[u8],
    ) -> Result<(), HarnessError> {
        let _gate = hold();
        let anchor = self.anchor(rel, false)?;
        let stat = final_stat(&anchor, "write workspace file", rel)?;
        if stat.is_symlink() {
            return Err(HarnessError::SymlinkRefused {
                path: rel.to_string(),
            });
        }
        if !stat.is_file() {
            return Err(HarnessError::NotRegularFile {
                path: rel.to_string(),
            });
        }
        if stat.len() > MAX_IO_BYTES as u64 {
            return Err(HarnessError::BoundsExceeded {
                path: rel.to_string(),
                found: stat.len(),
                max: MAX_IO_BYTES as u64,
            });
        }
        let file = anchor.open_verified(stat.identity(), rel)?;
        let mut current = Vec::new();
        (&file)
            .take(MAX_IO_BYTES as u64 + 1)
            .read_to_end(&mut current)
            .map_err(|error| io_error("read workspace file", anchor.path(), error))?;
        Self::digest_checked(rel, Some(expected_sha256), &current)?;
        anchor.commit_bytes(rel, bytes)
    }
}
