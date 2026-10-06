//! Guarded whole-file removal on the contained workspace: the delete the
//! recovery step needs, built on the same fresh-read admission, commit gate,
//! and anchored publication as the guarded write.
//!
//! `remove_guarded` unlinks an existing regular file only when its fresh
//! re-read hashes to the caller's observed digest — the undo of a file an
//! earlier edit created — so a removal can never take a file that is no
//! longer what the edit left. A missing file is `NotFound` (removal of
//! nothing is not success here: the caller decides); a changed file is
//! `ContentChanged`; a symlink is `SymlinkRefused`; a non-regular entry is
//! `NotRegularFile`; a file over the whole-file bound is `BoundsExceeded`.
//! Every refusal leaves the file byte-identical, and no temp file is ever
//! created: the only filesystem effect is the final unlink itself.
//!
//! The commit gate (`commit_gate`) is held from before the walk to after the
//! unlink, so two Saya writers serialize their verify-then-remove window —
//! the same optimistic stale-version guard, not an OS-level compare-and-swap:
//! no protection from a non-cooperating external program racing the final
//! unlink, the same documented limit the guarded write and patch carry.
//! Empty parent directories are deliberately left in place — removal never
//! deletes directories it did not create-track.

#[cfg(not(unix))]
mod path_impl;
#[cfg(unix)]
mod unix_impl;

#[cfg(test)]
#[path = "guarded_remove_tests.rs"]
mod guarded_remove_tests;

use super::contain::Workspace;
use crate::HarnessError;

impl Workspace {
    /// Removes the existing regular file `rel` only when its fresh re-read
    /// hashes to `expected_sha256` (lowercase hex SHA-256, e.g.
    /// `ReadFile::digest`). A missing file is [`HarnessError::NotFound`] —
    /// removal of nothing is not success, the caller decides; a changed file
    /// is [`HarnessError::ContentChanged`]. Every refusal leaves the file
    /// byte-identical, and the removal never touches empty parent
    /// directories or creates any temporary file.
    pub fn remove_guarded(&self, rel: &str, expected_sha256: &str) -> Result<(), HarnessError> {
        #[cfg(unix)]
        return self.remove_guarded_anchored(rel, expected_sha256);

        #[cfg(not(unix))]
        return self.remove_guarded_by_path(rel, expected_sha256);
    }
}
