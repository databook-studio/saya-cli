#[cfg(unix)]
use std::os::unix::ffi::OsStringExt as _;
use std::{collections::HashSet, ffi::OsString, path::PathBuf};

use super::repository::GitInventoryError;

/// The repository inventory's distinct-path bound. It sits above the
/// snapshot's 50k file bound so the snapshot bound reports its own measured
/// count rather than the inventory's.
pub(super) const MAX_INVENTORY_PATHS: usize = 60_000;
/// Pre-selection allowance for snapshot input paths; the selected-file count
/// of 50,000 is enforced by the snapshot manifest.
pub(super) const MAX_SNAPSHOT_INPUT_PATHS: usize = 60_000;
const MAX_INVENTORY_OUTPUT_BYTES: usize = 64 * 1024 * 1024;

pub(super) struct InventoryBudget {
    output_bytes: usize,
    path_limit: usize,
    paths: HashSet<Vec<u8>>,
}

impl Default for InventoryBudget {
    fn default() -> Self {
        Self {
            output_bytes: 0,
            path_limit: MAX_INVENTORY_PATHS,
            paths: HashSet::new(),
        }
    }
}

impl InventoryBudget {
    /// Start a budget whose distinct-path ceiling is `limit` instead of the
    /// inventory bound, for callers with their own pre-selection allowance.
    pub(super) fn with_path_limit(limit: usize) -> Self {
        Self {
            path_limit: limit,
            ..Self::default()
        }
    }

    pub(super) fn add_output(&mut self, bytes: &[u8]) -> Result<(), GitInventoryError> {
        let measured = self.output_bytes.saturating_add(bytes.len());
        if measured > MAX_INVENTORY_OUTPUT_BYTES {
            return Err(GitInventoryError::BoundExceeded {
                bound: "Git output bytes",
                measured: measured as u64,
                limit: MAX_INVENTORY_OUTPUT_BYTES as u64,
            });
        }
        self.output_bytes = measured;
        Ok(())
    }

    pub(super) fn add_path(&mut self, bytes: &[u8]) -> Result<(), GitInventoryError> {
        self.paths.insert(bytes.to_vec());
        if self.paths.len() > self.path_limit {
            return Err(GitInventoryError::BoundExceeded {
                bound: "distinct repository paths",
                measured: self.paths.len() as u64,
                limit: self.path_limit as u64,
            });
        }
        Ok(())
    }
}

pub(super) fn repository_path(bytes: &[u8]) -> Result<PathBuf, GitInventoryError> {
    if bytes.is_empty() || bytes.first() == Some(&b'/') {
        return Err(GitInventoryError::UnsafePath);
    }
    let components = bytes.strip_suffix(b"/").unwrap_or(bytes);
    if components.is_empty()
        || components
            .split(|byte| *byte == b'/')
            .any(|part| part.is_empty() || part == b"." || part == b"..")
    {
        return Err(GitInventoryError::UnsafePath);
    }
    path_from_git_bytes(bytes)
}

pub(super) fn path_from_git_bytes(bytes: &[u8]) -> Result<PathBuf, GitInventoryError> {
    #[cfg(unix)]
    {
        Ok(PathBuf::from(OsString::from_vec(bytes.to_vec())))
    }
    #[cfg(not(unix))]
    {
        let value = std::str::from_utf8(bytes).map_err(|_| GitInventoryError::MalformedOutput)?;
        Ok(PathBuf::from(OsString::from(value)))
    }
}
