//! The unix body of the range patch: the anchored walk, the no-follow
//! verified open, the bounded whole read, and the shared atomic commit —
//! the same operations the guarded whole-file write runs.

use std::io::Read as _;

use super::super::anchored::final_stat;
use super::super::commit_gate::hold;
use super::PATCH_MAX_FILE_BYTES;
use crate::workspace::contain::Workspace;
use crate::{HarnessError, io_error};

impl Workspace {
    pub(super) fn patch_range_anchored(
        &self,
        rel: &str,
        range: std::ops::Range<u64>,
        expected_len: u64,
        expected_digest: Option<&str>,
        replacement: &[u8],
    ) -> Result<(), HarnessError> {
        // The gate is held from before the fresh read to after the publish,
        // so no other Saya patch writer can interleave its own window.
        let _gate = hold();
        let anchor = self.anchor(rel, false)?;
        let stat = final_stat(&anchor, "patch workspace file", rel)?;
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
        if stat.len() > PATCH_MAX_FILE_BYTES {
            return Err(HarnessError::BoundsExceeded {
                path: rel.to_string(),
                found: stat.len(),
                max: PATCH_MAX_FILE_BYTES,
            });
        }
        let file = anchor.open_verified(stat.identity(), rel)?;
        let mut current = Vec::new();
        (&file)
            .take(PATCH_MAX_FILE_BYTES.saturating_add(1))
            .read_to_end(&mut current)
            .map_err(|error| io_error("read workspace file", anchor.path(), error))?;
        Self::digest_checked(rel, expected_digest, &current)?;
        let patched = Self::splice(rel, range, expected_len, replacement, &current)?;
        anchor.commit_bytes(rel, &patched)
    }
}
