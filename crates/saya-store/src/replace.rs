//! Atomic publish of staged session bytes (A012).
//!
//! [`publish_staged`] is the single replacement seam: the caller stages the
//! complete new bytes to `temp`, then this atomically publishes them at
//! `target`. The real [`AtomicReplace`] uses `fs::rename` on every platform —
//! on Windows that is `MoveFileExW(REPLACE_EXISTING)` with a
//! `SetFileInformationByHandle(FileRenameInfoEx)` fallback — so a failure
//! either leaves the existing target untouched or installs the complete new
//! file. The old copy-then-remove publish could truncate the target before
//! failing, losing the last good session. Tests inject a failing [`Replacer`]
//! to prove the last good record survives, without needing a Windows host.

use crate::StoreError;
use std::path::Path;

pub(crate) trait Replacer: Send + Sync {
    fn replace(&self, temp: &Path, target: &Path) -> Result<(), StoreError>;
}

pub(crate) struct AtomicReplace;

impl Replacer for AtomicReplace {
    fn replace(&self, temp: &Path, target: &Path) -> Result<(), StoreError> {
        std::fs::rename(temp, target).map_err(|_| StoreError::unavailable())
    }
}

/// A replacer that fails *without touching the target*: the contract the
/// real [`AtomicReplace`] satisfies and the old copy-then-remove publish
/// violated. Injected by tests to prove a failed publish preserves the last
/// good session on any platform.
#[cfg(test)]
pub(crate) struct FailingReplacer;

#[cfg(test)]
impl Replacer for FailingReplacer {
    fn replace(&self, temp: &Path, _target: &Path) -> Result<(), StoreError> {
        let _ = std::fs::remove_file(temp);
        Err(StoreError::unavailable())
    }
}

/// Atomically publishes staged `temp` bytes at `target` through `replacer`.
/// A failed publish leaves any existing target untouched.
pub(crate) fn publish_staged(
    temp: &Path,
    target: &Path,
    replacer: &dyn Replacer,
) -> Result<(), StoreError> {
    replacer.replace(temp, target)
}

/// Publishes staged bytes at `target` WITHOUT replacement, the exclusive
/// publish create uses: the target becomes a hard link to `temp`, which
/// fails with `AlreadyExists` when any file — including one a non-locking
/// writer created after the caller's check — occupies the target, so a
/// create can never overwrite. The temp link is always removed. When hard
/// links are unsupported on the filesystem, this falls back to the caller's
/// locked rename (exclusive among locking writers only, not against old
/// non-locking binaries).
pub(crate) fn publish_staged_no_replace(temp: &Path, target: &Path) -> Result<(), StoreError> {
    let published = match std::fs::hard_link(temp, target) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(StoreError::conflict())
        }
        Err(_) => std::fs::rename(temp, target).map_err(|_| StoreError::unavailable()),
    };
    let _ = std::fs::remove_file(temp);
    published
}

#[cfg(test)]
#[path = "filesystem_replace_tests.rs"]
mod tests;
