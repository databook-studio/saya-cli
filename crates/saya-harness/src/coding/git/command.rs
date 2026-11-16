use std::{
    fmt, io,
    path::{Path, PathBuf},
};

use saya_types::{GitObjectId, ManagedCheckoutId, ManagedCheckoutPath, redact};
use thiserror::Error;

use super::{compatibility::GitVersion, process::GitOutput};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum GitOperation {
    Read,
    Materialize,
    Remove,
}

impl GitOperation {
    pub(super) fn timeout(self) -> std::time::Duration {
        match self {
            Self::Read => std::time::Duration::from_secs(10),
            Self::Materialize | Self::Remove => std::time::Duration::from_secs(30),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitObjectFormat {
    Sha1,
    Sha256,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitConfigRisk {
    Include,
    Filter,
    PartialClone,
    PromisorRemote,
    SparseCheckout,
    SubmoduleRecursion,
    Submodule,
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum GitError {
    #[error("Git executable must be an installed absolute executable")]
    InvalidGitExecutable,
    #[error("Git service state must be a private, existing directory")]
    InvalidStateRoot,
    #[error("the platform cannot safely supervise Git process trees")]
    UnsupportedPlatform,
    #[error("path is not a Git working tree")]
    NotRepository,
    #[error("bare Git repositories cannot back a managed checkout")]
    BareRepository,
    #[error("repository identity changed since it was opened")]
    RepositoryChanged,
    #[error("repository uses unsupported materialization configuration: {0:?}")]
    UnsupportedConfiguration(GitConfigRisk),
    #[error(
        "the repository exceeds its {bound} bound: measured {measured}, allowed {limit}; narrow the repository or materialize a smaller commit"
    )]
    RepositoryBoundExceeded {
        bound: &'static str,
        measured: u64,
        limit: u64,
    },
    #[error("repository contains malformed Git metadata")]
    MalformedOutput,
    #[error("repository contains a Git tree entry that cannot be safely materialized")]
    UnsupportedTreeEntry,
    #[error("Git object format is not supported")]
    UnsupportedObjectFormat,
    #[error("the base commit is invalid or absent from this repository")]
    InvalidBaseCommit,
    #[error("managed checkout path is invalid")]
    InvalidManagedPath,
    #[error("managed checkout path is outside application state")]
    ManagedPathOutsideState,
    #[error("managed checkout path already exists")]
    ManagedPathAlreadyExists,
    #[error("managed checkout ownership does not match this Git service")]
    WorktreeOwnershipMismatch,
    #[error("managed checkout is dirty and cannot be removed")]
    WorktreeNotClean,
    #[error(
        "Git command failed: {operation:?} (exit {exit_code:?}){detail}; inspect the repository and its Git configuration, then retry"
    )]
    CommandFailed {
        operation: GitOperation,
        exit_code: Option<i32>,
        /// The operation's first non-empty stderr line, bounded and redacted:
        /// empty when Git printed none.
        detail: String,
    },
    #[error("Git output exceeded its bounded capture limit")]
    OutputLimitExceeded,
    #[error("Git command exceeded its deadline")]
    TimedOut,
    #[error("Git command was cancelled")]
    Cancelled,
    #[error("Git command may have partially changed repository state; reconcile before retrying")]
    MutationOutcomeUnknown {
        operation: GitOperation,
        #[source]
        source: Box<GitError>,
    },
    #[error("Git could not spawn the trusted executable")]
    Spawn(#[source] io::Error),
    #[error("Git process could not be waited or reaped")]
    Wait(#[source] io::Error),
    #[error("Git output could not be captured")]
    CaptureUnavailable,
    #[error("bounded Git request input could not be written")]
    InputUnavailable(#[source] io::Error),
    #[error("installed Git did not report a usable version")]
    UnsupportedGitVersion,
    #[error(
        "installed Git {found} is older than the minimum supported {minimum}; upgrade Git and retry"
    )]
    UnsupportedGitVersionTooOld {
        found: GitVersion,
        minimum: GitVersion,
    },
}

pub struct GitRepository {
    pub(super) root: PathBuf,
    pub(super) common_dir: PathBuf,
    pub(super) object_format: GitObjectFormat,
}

impl GitRepository {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn common_dir(&self) -> &Path {
        &self.common_dir
    }

    pub fn object_format(&self) -> GitObjectFormat {
        self.object_format
    }
}

impl Clone for GitRepository {
    fn clone(&self) -> Self {
        Self {
            root: self.root.clone(),
            common_dir: self.common_dir.clone(),
            object_format: self.object_format,
        }
    }
}

impl PartialEq for GitRepository {
    fn eq(&self, other: &Self) -> bool {
        self.root == other.root
            && self.common_dir == other.common_dir
            && self.object_format == other.object_format
    }
}

impl Eq for GitRepository {}

impl fmt::Debug for GitRepository {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GitRepository")
            .field("root", &"[canonical path]")
            .field("common_dir", &"[canonical path]")
            .field("object_format", &self.object_format)
            .finish()
    }
}

pub struct ManagedWorktree {
    pub(super) service_root: PathBuf,
    pub(super) repository: GitRepository,
    pub(super) checkout_id: ManagedCheckoutId,
    pub(super) path: ManagedCheckoutPath,
    pub(super) base_commit: GitObjectId,
}

impl ManagedWorktree {
    pub fn checkout_id(&self) -> &ManagedCheckoutId {
        &self.checkout_id
    }

    pub fn path(&self) -> &ManagedCheckoutPath {
        &self.path
    }

    pub fn base_commit(&self) -> &GitObjectId {
        &self.base_commit
    }
}

impl fmt::Debug for ManagedWorktree {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ManagedWorktree")
            .field("checkout_id", &self.checkout_id)
            .field("path", &"[private managed path]")
            .field("base_commit", &self.base_commit)
            .finish()
    }
}

pub(super) fn require_success(
    output: GitOutput,
    operation: GitOperation,
) -> Result<GitOutput, GitError> {
    if output.exit_code != Some(0) {
        return Err(GitError::CommandFailed {
            operation,
            exit_code: output.exit_code,
            detail: failure_detail(&output.stderr),
        });
    }
    Ok(output)
}

/// The first non-empty stderr line, redacted by the shared pass and bounded
/// to one short line: enough to name the cause without echoing a credential or
/// an unbounded dump.
pub(super) fn failure_detail(stderr: &[u8]) -> String {
    const MAX_DETAIL_CHARS: usize = 240;
    let text = String::from_utf8_lossy(stderr);
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    let redacted = redact(line);
    let bounded = redacted.chars().take(MAX_DETAIL_CHARS).collect::<String>();
    if bounded.is_empty() {
        String::new()
    } else {
        format!(": {bounded}")
    }
}

pub(super) fn unknown(operation: GitOperation, source: GitError) -> GitError {
    GitError::MutationOutcomeUnknown {
        operation,
        source: Box::new(source),
    }
}

#[cfg(test)]
mod failure_detail_tests {
    use super::failure_detail;

    #[test]
    fn the_first_stderr_line_names_the_cause_redacted_and_bounded() {
        assert_eq!(
            failure_detail(b"fatal: unable to read config\nlater line\n"),
            ": fatal: unable to read config"
        );
        assert_eq!(failure_detail(b"\n  \n"), "");
        let redacted = failure_detail(b"fatal: token=supersecretvalue\n");
        assert!(!redacted.contains("supersecretvalue"), "{redacted}");
        let long = failure_detail(format!("fatal: {}\n", "x".repeat(1000)).as_bytes());
        assert!(
            long.chars().count() <= 243,
            "{} chars",
            long.chars().count()
        );
    }

    #[test]
    fn a_failed_command_display_names_its_cause_and_remedy() {
        let error = super::GitError::CommandFailed {
            operation: super::GitOperation::Read,
            exit_code: Some(128),
            detail: failure_detail(b"fatal: not a git repository\n"),
        };
        let text = error.to_string();
        assert!(text.contains("fatal: not a git repository"), "{text}");
        assert!(text.contains("inspect the repository"), "{text}");
    }
}
