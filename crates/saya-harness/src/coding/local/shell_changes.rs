//! Shell-made changes in an in-place root: what the user's own shell (or an
//! editor, or a build) changed since the job started, read from Git status.
//!
//! A job that writes the user's folder cannot see shell edits through its
//! before-image store: the store only holds what its own tools touched. This
//! module reads the root's Git working-tree status at start and at review and
//! reports the difference, excluding every path a tool touched. Pre-existing
//! dirty files are not the job's changes; a dirty-at-start path whose size or
//! mtime moved is reported as `modified_since_start`.
//!
//! Every failure is `Unrecorded`: a plain folder, a damaged `.git`, a local
//! config with filters, or a listing over its bound never fails start, show or
//! review. Nothing here is authority and nothing here is rewound.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use saya_agent::CancellationToken;
use saya_types::GitObjectId;
use serde::{Deserialize, Serialize};

use crate::coding::git::{GitError, GitService, StatusKind};

/// The most dirty paths the start snapshot keeps. Beyond this the snapshot is
/// `truncated` and every later read is `Unrecorded`.
pub const MAX_SHELL_START_ENTRIES: usize = 4096;
/// The most shell changes one review names. Beyond this the list is truncated.
pub const MAX_SHELL_CHANGES: usize = 4096;
/// The start snapshot's own stored byte bound.
pub const MAX_SHELL_START_BYTES: usize = 1024 * 1024;
/// The most shell-changed files a review hashes; beyond it, size and mtime.
pub const MAX_SHELL_HASH_FILES: usize = 512;
/// The largest shell-changed file a review hashes.
pub const MAX_SHELL_HASH_FILE_BYTES: u64 = 8 * 1024 * 1024;
/// The most paths one shell diff names, and the diff's byte cap.
pub const MAX_SHELL_DIFF_PATHS: usize = 128;
pub const MAX_SHELL_DIFF_BYTES: usize = 64 * 1024;

/// How a shell change appears in the working tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellChangeKind {
    Added,
    Modified,
    Deleted,
    /// A path already dirty when the job started whose size or mtime moved.
    ModifiedSinceStart,
}

/// One shell-made change, repository-relative.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellChange {
    pub path: String,
    pub kind: ShellChangeKind,
}

/// Why shell changes could not be recorded. Closed and serialized in snake
/// case; the text never carries a path or an error message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellUnrecorded {
    NotAGitRepository,
    GitUnavailable,
    GitConfigFilters,
    TooManyDirtyPaths,
    GitFailed,
}

impl ShellUnrecorded {
    pub fn reason(self) -> &'static str {
        match self {
            Self::NotAGitRepository => "not_a_git_repository",
            Self::GitUnavailable => "git_unavailable",
            Self::GitConfigFilters => "git_config_filters",
            Self::TooManyDirtyPaths => "too_many_dirty_paths",
            Self::GitFailed => "git_failed",
        }
    }
}

/// One dirty path at job start, with the size and mtime it held then.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartEntry {
    pub path: String,
    pub size: u64,
    pub mtime_ns: u64,
    pub kind: ShellChangeKind,
}

/// The start snapshot written to `<records>/git-start.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ShellStart {
    Recorded {
        /// The HEAD oid at start; `None` when the branch was unborn.
        head: Option<String>,
        entries: Vec<StartEntry>,
        truncated: bool,
    },
    Unrecorded {
        reason: ShellUnrecorded,
    },
}

/// The shell changes between job start and now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ShellChanges {
    Recorded {
        paths: Vec<ShellChange>,
        commits_since_start: u32,
        truncated: bool,
    },
    Unrecorded {
        reason: ShellUnrecorded,
    },
}

impl ShellChanges {
    pub fn unrecorded(reason: ShellUnrecorded) -> Self {
        Self::Unrecorded { reason }
    }
}

/// How long a review's status read may take. The listing is bounded by the
/// Git service's own output and path limits; this only stops a wedged Git.
const SHELL_READ_DEADLINE: Duration = Duration::from_secs(10);

/// Reads the working-tree status at job start. Never fails: every Git failure
/// becomes an `Unrecorded` snapshot with its closed reason.
pub async fn capture_start(
    service: &GitService,
    root: &Path,
    cancellation: &CancellationToken,
    deadline: Duration,
) -> ShellStart {
    let entries = match service
        .working_tree_status(root, cancellation, deadline)
        .await
    {
        Ok(entries) => entries,
        Err(error) => return unrecorded(&error),
    };
    let head = match service.head_oid(root, cancellation, deadline).await {
        Ok(head) => head,
        Err(error) => return unrecorded(&error),
    };
    let mut start = Vec::with_capacity(entries.len().min(MAX_SHELL_START_ENTRIES));
    for entry in entries.iter().take(MAX_SHELL_START_ENTRIES) {
        let Some(path) = entry.path.to_str() else {
            return unrecorded(&GitError::MalformedOutput);
        };
        if is_scratch(path) {
            continue;
        }
        let (size, mtime_ns) = stat(root, &entry.path);
        start.push(StartEntry {
            path: path.to_owned(),
            size,
            mtime_ns,
            kind: kind(entry.kind),
        });
    }
    ShellStart::Recorded {
        head: head.map(|head| head.as_str().to_owned()),
        entries: start,
        truncated: entries.len() > MAX_SHELL_START_ENTRIES,
    }
}

/// The shell changes between `start` and now, excluding every `touched` path.
/// Never fails: every Git failure becomes `Unrecorded`.
pub async fn shell_changes(
    service: &GitService,
    root: &Path,
    start: &ShellStart,
    touched: &HashSet<String>,
    cancellation: &CancellationToken,
) -> ShellChanges {
    let (head, entries, truncated) = match start {
        ShellStart::Recorded {
            head,
            entries,
            truncated,
        } => (head, entries, *truncated),
        ShellStart::Unrecorded { reason } => {
            return ShellChanges::unrecorded(*reason);
        }
    };
    // A start snapshot that could not hold every dirty path cannot attribute
    // anything later: the excluded set is unknown.
    if truncated {
        return ShellChanges::unrecorded(ShellUnrecorded::TooManyDirtyPaths);
    }
    let current = match service
        .working_tree_status(root, cancellation, SHELL_READ_DEADLINE)
        .await
    {
        Ok(current) => current,
        Err(error) => return unrecorded_changes(&error),
    };
    let dirty_at_start: HashMap<&str, &StartEntry> = entries
        .iter()
        .map(|entry| (entry.path.as_str(), entry))
        .collect();
    let mut paths = Vec::new();
    for entry in &current {
        let Some(path) = entry.path.to_str() else {
            return unrecorded_changes(&GitError::MalformedOutput);
        };
        if is_scratch(path) || tool_owned(path, touched) {
            continue;
        }
        match dirty_at_start.get(path) {
            Some(previous) => {
                let (size, mtime_ns) = stat(root, &entry.path);
                if size != previous.size || mtime_ns != previous.mtime_ns {
                    paths.push(ShellChange {
                        path: path.to_owned(),
                        kind: ShellChangeKind::ModifiedSinceStart,
                    });
                }
            }
            None => paths.push(ShellChange {
                path: path.to_owned(),
                kind: kind(entry.kind),
            }),
        }
    }
    paths.sort_by(|left, right| left.path.cmp(&right.path));
    let truncated = paths.len() > MAX_SHELL_CHANGES;
    paths.truncate(MAX_SHELL_CHANGES);
    let commits_since_start = match head
        .as_deref()
        .and_then(|head| GitObjectId::parse(head).ok())
    {
        Some(base) => service
            .count_since(root, &base, cancellation, SHELL_READ_DEADLINE)
            .await
            .unwrap_or(0),
        None => 0,
    };
    ShellChanges::Recorded {
        paths,
        commits_since_start,
        truncated,
    }
}

/// A bounded unified diff of the recorded shell changes against the start
/// HEAD, or `None` when the branch was unborn, nothing was recorded, or Git
/// could not produce it. The text is informational; review never fails on it.
pub async fn shell_diff(
    service: &GitService,
    root: &Path,
    start: &ShellStart,
    changes: &ShellChanges,
    cancellation: &CancellationToken,
) -> Option<String> {
    let ShellStart::Recorded {
        head: Some(head), ..
    } = start
    else {
        return None;
    };
    let ShellChanges::Recorded { paths, .. } = changes else {
        return None;
    };
    let base = GitObjectId::parse(head).ok()?;
    let names: Vec<String> = paths
        .iter()
        .take(MAX_SHELL_DIFF_PATHS)
        .map(|change| change.path.clone())
        .collect();
    if names.is_empty() {
        return None;
    }
    let bytes = service
        .diff_paths(root, &base, &names, cancellation, SHELL_READ_DEADLINE)
        .await
        .ok()?;
    let bytes = &bytes[..bytes.len().min(MAX_SHELL_DIFF_BYTES)];
    Some(String::from_utf8_lossy(bytes).into_owned())
}

fn kind(kind: StatusKind) -> ShellChangeKind {
    match kind {
        StatusKind::Added => ShellChangeKind::Added,
        StatusKind::Modified => ShellChangeKind::Modified,
        StatusKind::Deleted => ShellChangeKind::Deleted,
    }
}

/// The job's own scratch tree is never a shell change: it is created and
/// removed around the job, and it lives inside the user's folder.
fn is_scratch(path: &str) -> bool {
    path == ".saya" || path.starts_with(".saya/")
}

/// Whether the job's own tools account for this path. A collapsed untracked
/// directory counts as the job's when any touched file sits under it: without
/// walking the tree its other contents cannot be told apart from tool writes.
fn tool_owned(path: &str, touched: &HashSet<String>) -> bool {
    if touched.contains(path) {
        return true;
    }
    let prefix = format!("{path}/");
    touched
        .iter()
        .any(|candidate| candidate.starts_with(&prefix))
}

fn stat(root: &Path, path: &Path) -> (u64, u64) {
    let Ok(metadata) = std::fs::symlink_metadata(root.join(path)) else {
        return (0, 0);
    };
    let mtime_ns = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|elapsed| u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX))
        .unwrap_or(0);
    (metadata.len(), mtime_ns)
}

fn unrecorded(error: &GitError) -> ShellStart {
    ShellStart::Unrecorded {
        reason: reason(error),
    }
}

fn unrecorded_changes(error: &GitError) -> ShellChanges {
    ShellChanges::unrecorded(reason(error))
}

fn reason(error: &GitError) -> ShellUnrecorded {
    match error {
        GitError::NotRepository => ShellUnrecorded::NotAGitRepository,
        GitError::UnsupportedConfiguration(_) => ShellUnrecorded::GitConfigFilters,
        GitError::RepositoryBoundExceeded { .. } => ShellUnrecorded::TooManyDirtyPaths,
        GitError::InvalidGitExecutable
        | GitError::UnsupportedPlatform
        | GitError::UnsupportedGitVersion
        | GitError::UnsupportedGitVersionTooOld { .. }
        | GitError::Spawn(_) => ShellUnrecorded::GitUnavailable,
        _ => ShellUnrecorded::GitFailed,
    }
}
