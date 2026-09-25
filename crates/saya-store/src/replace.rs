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

#[cfg(test)]
#[path = "filesystem_replace_tests.rs"]
mod tests;
