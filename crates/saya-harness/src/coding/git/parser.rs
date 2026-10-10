#[cfg(unix)]
use std::os::unix::ffi::OsStringExt as _;
use std::{collections::HashSet, ffi::OsString, path::PathBuf};

use super::repository::GitInventoryError;

pub(super) const MAX_INVENTORY_PATHS: usize = 20_000;
const MAX_INVENTORY_OUTPUT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Default)]
pub(super) struct InventoryBudget {
    output_bytes: usize,
    paths: HashSet<Vec<u8>>,
}

impl InventoryBudget {
    pub(super) fn add_output(&mut self, bytes: &[u8]) -> Result<(), GitInventoryError> {
        self.output_bytes = self
            .output_bytes
            .checked_add(bytes.len())
            .filter(|total| *total <= MAX_INVENTORY_OUTPUT_BYTES)
            .ok_or(GitInventoryError::TooLarge)?;
        Ok(())
    }

    pub(super) fn add_path(&mut self, bytes: &[u8]) -> Result<(), GitInventoryError> {
        self.paths.insert(bytes.to_vec());
        if self.paths.len() > MAX_INVENTORY_PATHS {
            return Err(GitInventoryError::TooLarge);
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
