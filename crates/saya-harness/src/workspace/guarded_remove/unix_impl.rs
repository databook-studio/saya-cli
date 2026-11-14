//! The unix body of the guarded removal: the anchored walk without parent
//! creation, the no-follow verified open, the bounded whole read, the digest
//! compare — then the unlink anchored to the parent descriptor the walk
//! validated, never a re-resolved path removal.

use std::io::Read as _;

use super::super::anchored::final_stat;
use super::super::commit_gate::hold;
use super::super::contain::MAX_IO_BYTES;
use crate::workspace::contain::Workspace;
use crate::{HarnessError, io_error};

impl Workspace {
    /// The unix removal: the same admission the guarded replace runs — the
    /// stat, the verified no-follow open, the bounded whole read, the digest
    /// compare — then the anchored unlink of the final component.
    pub(super) fn remove_guarded_anchored(
        &self,
        rel: &str,
        expected_sha256: &str,
    ) -> Result<(), HarnessError> {
        let _gate = hold();
        let anchor = self.anchor(rel, false)?;
        let stat = final_stat(&anchor, "remove workspace file", rel)?;
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
        anchor
            .unlink_final()
            .map_err(|error| io_error("remove workspace file", anchor.path(), error))
    }
}
