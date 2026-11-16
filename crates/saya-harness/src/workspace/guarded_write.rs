//! Guarded whole-file creation and replacement on the contained workspace:
//! the two whole-file operations built on the same fresh-read admission,
//! commit gate, and atomic publication as the range patch.
//!
//! - `create_new` is create-if-absent. Inside the gate it walks with parent
//!   creation allowed, refuses any existing final entry, and only then
//!   publishes; a refused create leaves the existing entry byte-identical.
//! - `write_guarded` replaces an existing regular file only when its fresh
//!   re-read hashes to the caller's observed digest; a missing file is
//!   `NotFound` (creation is `create_new`'s job) and a changed file is
//!   `ContentChanged`. Every refusal leaves the original bytes.
//!
//! Both hold the process-wide commit gate (`commit_gate`) from before the
//! walk or fresh read until after publication. The existence check in
//! `create_new` is therefore an optimistic one among Saya's own serialized
//! writers — not an OS atomic exclusive-create, and no protection from a
//! non-cooperating external program racing the final rename; the same
//! documented limit the guarded patch carries.
//!
//! The platform bodies live in `unix_impl` and `path_impl`, mirroring how
//! the whole-file write and the range patch split.

#[cfg(not(unix))]
mod path_impl;
#[cfg(unix)]
mod unix_impl;

use super::contain::{MAX_IO_BYTES, Workspace};
use crate::HarnessError;

impl Workspace {
    /// Creates `rel` with `bytes` only when nothing names it yet; missing
    /// parent directories are created. An existing final entry — file or
    /// directory — refuses with [`HarnessError::AlreadyExists`] (the shared
    /// walk refuses a symlink as `SymlinkRefused`) and is left untouched.
    /// Content over the whole-file write bound refuses before any
    /// filesystem contact.
    pub fn create_new(&self, rel: &str, bytes: &[u8]) -> Result<(), HarnessError> {
        if bytes.len() > MAX_IO_BYTES {
            return Err(HarnessError::BoundsExceeded {
                path: rel.to_string(),
                found: bytes.len() as u64,
                max: MAX_IO_BYTES as u64,
            });
        }
        #[cfg(unix)]
        return self.create_new_anchored(rel, bytes);

        #[cfg(not(unix))]
        return self.create_new_by_path(rel, bytes);
    }

    /// Replaces the existing file `rel` with `bytes` only when its fresh
    /// re-read hashes to `expected_sha256` (lowercase hex SHA-256, e.g.
    /// `ReadFile::digest`). A missing file is [`HarnessError::NotFound`]; a
    /// same-size change is [`HarnessError::ContentChanged`]. Atomic: every
    /// refusal leaves the original bytes, and a crash before the rename
    /// never leaves a partial file.
    pub fn write_guarded(
        &self,
        rel: &str,
        expected_sha256: &str,
        bytes: &[u8],
    ) -> Result<(), HarnessError> {
        if bytes.len() > MAX_IO_BYTES {
            return Err(HarnessError::BoundsExceeded {
                path: rel.to_string(),
                found: bytes.len() as u64,
                max: MAX_IO_BYTES as u64,
            });
        }
        #[cfg(unix)]
        return self.write_guarded_anchored(rel, expected_sha256, bytes);

        #[cfg(not(unix))]
        return self.write_guarded_by_path(rel, expected_sha256, bytes);
    }
}
