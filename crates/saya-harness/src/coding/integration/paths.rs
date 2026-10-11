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

    /// Conservatively detects case-folded equal or component-ancestor paths.
    /// Component sorting keeps descendants contiguous, unlike raw string order.
    pub fn overlaps(&self, other: &Self) -> bool {
        // Unicode aliases depend on filesystem normalization/case rules. Without
        // a platform identity proof, serialize any assignment containing them.
        if self
            .entries
            .iter()
            .chain(&other.entries)
            .any(|path| !path.is_ascii())
        {
            return true;
        }
        let fold = |paths: &[String]| {
            let mut paths = paths
                .iter()
                .map(|path| path.to_lowercase())
                .collect::<Vec<_>>();
            paths.sort_by(|a, b| a.split('/').cmp(b.split('/')));
            paths
        };
        let left = fold(&self.entries);
        let right = fold(&other.entries);
        let (mut i, mut j) = (0, 0);
        while i < left.len() && j < right.len() {
            let a = &left[i];
            let b = &right[j];
            let contains = |parent: &str, child: &str| {
                child == parent
                    || child
                        .strip_prefix(parent)
                        .is_some_and(|tail| tail.starts_with('/'))
            };
            if contains(a, b) || contains(b, a) {
                return true;
            }
            if a.split('/').cmp(b.split('/')).is_lt() {
                i += 1;
            } else {
                j += 1;
            }
        }
        false
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
