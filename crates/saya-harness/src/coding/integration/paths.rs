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
    /// The whole checkout rather than a list of paths: every path is covered.
    /// A candidate's scope is the checkout itself, and the operator's review
    /// and apply is the check.
    whole: bool,
}

impl fmt::Debug for AssignedPaths {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AssignedPaths")
            .field("count", &self.entries.len())
            .field("whole", &self.whole)
            .finish()
    }
}

#[non_exhaustive]
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum AssignedPathsError {
    #[error("assigned path set is empty")]
    Empty,
    #[error(
        "the write path set has {measured} entries; the limit is {limit}; pass `.` to cover the whole repository"
    )]
    TooMany { measured: usize, limit: usize },
    #[error(
        "a write path entry is {measured} bytes; the limit is {limit} bytes per entry and {total_limit} bytes in total; shorten the paths or pass `.` to cover the whole repository"
    )]
    EntryTooLarge {
        path: String,
        measured: usize,
        limit: usize,
        total_limit: usize,
    },
    #[error(
        "write paths total {measured} bytes; the limit is {limit}; shorten the paths or pass `.` to cover the whole repository"
    )]
    TotalTooLarge { measured: usize, limit: usize },
    #[error(
        "a write path entry is not a safe repository-relative path; name a path like `src/lib.rs`, or pass `.` to cover the whole repository"
    )]
    UnsafePath { path: String },
    #[error("the write path set lists a path more than once; list each path once")]
    Duplicate { path: String },
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
        // `.` states the whole checkout. It covers every path, so any other
        // entries beside it are redundant.
        if entries.iter().any(|entry| entry == ".") {
            return Ok(Self::whole_checkout());
        }
        if entries.len() > MAX_ASSIGNED_PATHS {
            return Err(AssignedPathsError::TooMany {
                measured: entries.len(),
                limit: MAX_ASSIGNED_PATHS,
            });
        }
        let mut total = 0usize;
        for entry in &entries {
            if entry.len() > MAX_ASSIGNED_PATH_BYTES {
                return Err(AssignedPathsError::EntryTooLarge {
                    path: entry.clone(),
                    measured: entry.len(),
                    limit: MAX_ASSIGNED_PATH_BYTES,
                    total_limit: MAX_ASSIGNED_PATHS_TOTAL_BYTES,
                });
            }
            total = total.saturating_add(entry.len());
            if total > MAX_ASSIGNED_PATHS_TOTAL_BYTES {
                return Err(AssignedPathsError::TotalTooLarge {
                    measured: total,
                    limit: MAX_ASSIGNED_PATHS_TOTAL_BYTES,
                });
            }
        }
        for entry in &entries {
            if SnapshotPath::parse(entry.clone()).is_err() {
                return Err(AssignedPathsError::UnsafePath {
                    path: entry.clone(),
                });
            }
        }
        let mut folded: HashSet<String> = HashSet::new();
        for entry in &entries {
            if !folded.insert(entry.to_lowercase()) {
                return Err(AssignedPathsError::Duplicate {
                    path: entry.clone(),
                });
            }
        }
        entries.sort();
        Ok(Self {
            entries,
            whole: false,
        })
    }

    /// The whole checkout as one assignment: every path in it is covered. The
    /// root and its helpers may change any file in the checkout; the
    /// operator's review and apply is
    /// the check. A candidate built from the working checkout uses this, so the
    /// receipt keeps binding the delta to the checkout and its byte bound while
    /// the path list restricts nothing.
    pub fn whole_checkout() -> Self {
        Self {
            entries: Vec::new(),
            whole: true,
        }
    }

    pub fn contains(&self, path: &str) -> bool {
        if self.whole {
            return SnapshotPath::parse(path.to_owned()).is_ok();
        }
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

    /// The number of explicitly named entries: 0 for a whole-checkout
    /// assignment, which names none because it covers all of them.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// The parsed entries, sorted lexicographically; empty for a whole-checkout
    /// assignment. Read-only enumeration so consumers can check per-entry
    /// membership with `contains` without writing their own path matching.
    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        !self.whole && self.entries.is_empty()
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
