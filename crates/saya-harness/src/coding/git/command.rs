use std::{
    fmt, io,
    path::{Path, PathBuf},
};

use saya_types::{GitObjectId, ManagedCheckoutId, ManagedCheckoutPath};
use thiserror::Error;

use super::process::GitOutput;

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
    WorktreeConfig,
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
    #[error("repository inventory exceeds the bounded Git service limit")]
    RepositoryTooLarge,
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
    #[error("Git command failed: {operation:?} (exit {exit_code:?})")]
    CommandFailed {
        operation: GitOperation,
        exit_code: Option<i32>,
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
    #[error("installed Git lacks the required no-lazy-fetch support")]
    UnsupportedGitVersion,
    #[error("Git did not report a valid version")]
    InvalidGitVersion,
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
        });
    }
    Ok(output)
}

pub(super) fn unknown(operation: GitOperation, source: GitError) -> GitError {
    GitError::MutationOutcomeUnknown {
        operation,
        source: Box::new(source),
    }
}
