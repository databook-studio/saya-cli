//! Assigned-path set parsing, membership matching, and case-fold collision detection.

use std::collections::HashSet;
use std::fmt;

use crate::coding::snapshot::SnapshotPath;

pub const MAX_ASSIGNED_PATHS: usize = 128;
pub const MAX_ASSIGNED_PATH_BYTES: usize = 512;
pub const MAX_ASSIGNED_PATHS_TOTAL_BYTES: usize = 32 * 1024;

#[derive(Clone, PartialEq, Eq)]
pub struct AssignedPaths {
    entries: Vec<String>,
}

impl fmt::Debug for AssignedPaths {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AssignedPaths")
            .field("count", &self.entries.len())
            .finish()
    }
}

#[non_exhaustive]
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum AssignedPathsError {
    #[error("assigned path set is empty")]
    Empty,
    #[error("assigned path set exceeds the maximum entry count")]
    TooMany,
    #[error("assigned path entries exceed the size bounds")]
    TooLarge,
    #[error("assigned path entry is not a safe relative path")]
    UnsafePath,
    #[error("assigned path set contains duplicate entries")]
    Duplicate,
}

impl AssignedPaths {
    pub fn parse<I: IntoIterator<Item = String>>(paths: I) -> Result<Self, AssignedPathsError> {
        let mut entries: Vec<String> = paths
            .into_iter()
            .by_ref()
            .take(MAX_ASSIGNED_PATHS + 1)
            .collect();
        if entries.is_empty() {
            return Err(AssignedPathsError::Empty);
        }
        if entries.len() > MAX_ASSIGNED_PATHS {
            return Err(AssignedPathsError::TooMany);
        }
        let mut total = 0usize;
        for entry in &entries {
            if entry.len() > MAX_ASSIGNED_PATH_BYTES {
                return Err(AssignedPathsError::TooLarge);
            }
            total = total.saturating_add(entry.len());
            if total > MAX_ASSIGNED_PATHS_TOTAL_BYTES {
                return Err(AssignedPathsError::TooLarge);
            }
        }
        for entry in &entries {
            if SnapshotPath::parse(entry.clone()).is_err() {
                return Err(AssignedPathsError::UnsafePath);
            }
        }
        let mut folded: HashSet<String> = HashSet::new();
        for entry in &entries {
            if !folded.insert(entry.to_lowercase()) {
                return Err(AssignedPathsError::Duplicate);
            }
        }
        entries.sort();
        Ok(Self { entries })
    }

    pub fn contains(&self, path: &str) -> bool {
        if SnapshotPath::parse(path.to_owned()).is_err() {
            return false;
        }
        self.entries.iter().any(|entry| {
            path == entry
                || (path.len() > entry.len()
                    && path.starts_with(entry.as_str())
                    && path.as_bytes()[entry.len()] == b'/')
        })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

pub fn casefold_collision<'a>(paths: impl IntoIterator<Item = &'a str>) -> bool {
    let distinct: HashSet<&'a str> = paths.into_iter().collect();
    let mut folded: HashSet<String> = HashSet::new();
    for path in &distinct {
        if !folded.insert((*path).to_lowercase()) {
            return true;
        }
    }
    false
}
