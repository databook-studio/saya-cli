use std::ffi::OsString;

use saya_types::{GitObjectId, ManagedCheckoutPath};

use super::command::{GitError, GitOperation};
use crate::coding::snapshot::MAX_SNAPSHOT_FILE_BYTES;

const MAX_GIT_STDOUT_BYTES: usize = 64 * 1024 * 1024;
const MAX_GIT_STDERR_BYTES: usize = 1024 * 1024;
/// The attribute scan sends every snapshot path to Git on stdin; 50k paths
/// with long names must fit, so this matches the stdout bound.
const MAX_GIT_STDIN_BYTES: usize = 32 * 1024 * 1024;

const SNAPSHOT_BLOB_STDOUT_BYTES: usize = match MAX_SNAPSHOT_FILE_BYTES.checked_add(1024) {
    Some(total) => total as usize,
    None => panic!("snapshot blob stdout limit overflows"),
};

#[derive(Debug, Clone, Copy)]
pub(super) enum GitRequest<'a> {
    Version,
    IsBare,
    RepositoryRoot,
    GitDir,
    CommonDir,
    ObjectFormat,
    LocalConfig,
    WorktreeConfig,
    VerifyCommit(&'a GitObjectId),
    VerifyCommits(&'a [GitObjectId]),
    CheckFilterAttributes {
        source: Option<&'a GitObjectId>,
        paths: &'a [String],
    },
    InitializeIndex(&'a GitObjectId),
    IndexEntries,
    IndexStageEntries,
    TreeEntries(&'a GitObjectId),
    ReadBlob(&'a GitObjectId),
    SnapshotBlob(&'a GitObjectId),
    Head,
    Branch,
    LogSince {
        base: &'a GitObjectId,
        limit: usize,
    },
    UnstagedNames,
    StagedNames,
    Status,
    /// The working tree's changed paths, without ignore records and without
    /// headers: the edit-time shell-change baseline. `normal` untracked mode
    /// keeps an untracked directory to one record, and a dirty submodule never
    /// appears. `--no-optional-locks` keeps the index lock untouched.
    StatusChangedZ,
    /// The raw diff of up to a bounded path list against one commit.
    DiffPaths {
        base: &'a GitObjectId,
        paths: &'a [String],
    },
    /// How many commits `base..HEAD` holds.
    CountSince {
        base: &'a GitObjectId,
    },
    /// Ignored files and ignored directory markers, one record each: the
    /// `normal` untracked mode keeps an ignored directory bounded to a single
    /// entry, so a build tree cannot flood the listing.
    IgnoredPaths,
    CheckIgnored(&'a str),
    WorktreeListPorcelainZ,
    WorktreeListPorcelain,
    StatusPorcelainV2BranchZ,
    AddWorktree {
        path: &'a ManagedCheckoutPath,
        base: &'a GitObjectId,
    },
    AddWorktreeNoCheckout {
        path: &'a ManagedCheckoutPath,
        base: &'a GitObjectId,
    },
    RemoveWorktree {
        path: &'a ManagedCheckoutPath,
    },
}

impl GitRequest<'_> {
    pub(super) fn kind(self) -> &'static str {
        match self {
            Self::Version => "version",
            Self::IsBare => "is_bare",
            Self::RepositoryRoot => "repository_root",
            Self::GitDir => "git_dir",
            Self::CommonDir => "common_dir",
            Self::ObjectFormat => "object_format",
            Self::LocalConfig => "local_config",
            Self::WorktreeConfig => "worktree_config",
            Self::VerifyCommit(_) => "verify_commit",
            Self::VerifyCommits(_) => "verify_commits",
            Self::CheckFilterAttributes { .. } => "check_filter_attributes",
            Self::InitializeIndex(_) => "initialize_index",
            Self::IndexEntries => "index_entries",
            Self::IndexStageEntries => "index_stage_entries",
            Self::TreeEntries(_) => "tree_entries",
            Self::ReadBlob(_) => "read_blob",
            Self::SnapshotBlob(_) => "snapshot_blob",
            Self::Head => "head",
            Self::Branch => "branch",
            Self::LogSince { .. } => "log_since",
            Self::UnstagedNames => "unstaged_names",
            Self::StagedNames => "staged_names",
            Self::Status => "status",
            Self::StatusChangedZ => "status_changed",
            Self::DiffPaths { .. } => "diff_paths",
            Self::CountSince { .. } => "count_since",
            Self::IgnoredPaths => "ignored_paths",
            Self::CheckIgnored(_) => "check_ignored",
            Self::WorktreeListPorcelainZ | Self::WorktreeListPorcelain => "worktree_list",
            Self::StatusPorcelainV2BranchZ => "status_branch",
            Self::AddWorktree { .. } => "add_worktree",
            Self::AddWorktreeNoCheckout { .. } => "add_worktree_no_checkout",
            Self::RemoveWorktree { .. } => "remove_worktree",
        }
    }

    pub(super) fn operation(self) -> GitOperation {
        match self {
            Self::AddWorktree { .. }
            | Self::AddWorktreeNoCheckout { .. }
            | Self::InitializeIndex(_) => GitOperation::Materialize,
            Self::RemoveWorktree { .. } => GitOperation::Remove,
            _ => GitOperation::Read,
        }
    }

    pub(super) fn output_limits(self) -> (usize, usize) {
        match self {
            Self::Version => (4096, 64 * 1024),
            Self::CheckIgnored(_) => (32, 4096),
            Self::DiffPaths { .. } => (64 * 1024, 64 * 1024),
            Self::CountSince { .. } => (32, 4096),
            Self::LogSince { .. } => (256 * 1024, 64 * 1024),
            Self::LocalConfig | Self::WorktreeConfig | Self::ReadBlob(_) => {
                (1024 * 1024, 64 * 1024)
            }
            Self::AddWorktree { .. }
            | Self::AddWorktreeNoCheckout { .. }
            | Self::RemoveWorktree { .. } => (1024 * 1024, 1024 * 1024),
            Self::SnapshotBlob(_) => (SNAPSHOT_BLOB_STDOUT_BYTES, 64 * 1024),
            _ => (MAX_GIT_STDOUT_BYTES, MAX_GIT_STDERR_BYTES),
        }
    }

    pub(super) fn arguments(self) -> Vec<OsString> {
        let strings: Vec<&str> = match self {
            Self::Version => vec!["--version"],
            Self::IsBare => vec!["rev-parse", "--is-bare-repository"],
            Self::RepositoryRoot => vec!["rev-parse", "--show-toplevel"],
            Self::GitDir => vec!["rev-parse", "--absolute-git-dir"],
            Self::CommonDir => vec!["rev-parse", "--git-common-dir"],
            Self::ObjectFormat => vec!["rev-parse", "--show-object-format"],
            Self::LocalConfig => vec!["config", "--local", "--null", "--list", "--no-includes"],
            // With `extensions.worktreeconfig` enabled, per-worktree settings
            // live outside `--local`; without it Git falls back to `--local`.
            Self::WorktreeConfig => {
                vec!["config", "--worktree", "--null", "--list", "--no-includes"]
            }
            Self::InitializeIndex(_) => vec!["read-tree"],
            Self::IndexEntries => vec!["ls-files", "-v", "-z"],
            Self::IndexStageEntries => vec!["ls-files", "--stage", "-z"],
            Self::Head => vec!["rev-parse", "--verify", "HEAD^{commit}"],
            Self::Branch => vec!["rev-parse", "--abbrev-ref", "HEAD"],
            Self::LogSince { .. } => vec![
                "log",
                "--no-decorate",
                "--no-show-signature",
                "--format=%H%x00%s",
            ],
            Self::UnstagedNames => {
                vec![
                    "diff",
                    "--name-only",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--",
                ]
            }
            Self::StagedNames => vec![
                "diff",
                "--cached",
                "--name-only",
                "--no-ext-diff",
                "--no-textconv",
                "--",
            ],
            Self::Status => vec![
                "status",
                "--porcelain=v2",
                "-z",
                "--untracked-files=all",
                "--ignored=traditional",
            ],
            Self::StatusChangedZ => vec![
                "--no-optional-locks",
                "status",
                "--porcelain=v2",
                "-z",
                "--untracked-files=normal",
                "--ignore-submodules=dirty",
            ],
            Self::DiffPaths { .. } => vec!["diff", "--no-ext-diff", "--no-textconv", "--no-color"],
            Self::CountSince { .. } => vec!["rev-list", "--count"],
            Self::IgnoredPaths => vec![
                "status",
                "--porcelain=v2",
                "-z",
                "--untracked-files=normal",
                "--ignored=traditional",
            ],
            Self::CheckIgnored(_) => vec!["check-ignore", "-q", "--"],
            Self::WorktreeListPorcelainZ => vec!["worktree", "list", "--porcelain", "-z"],
            Self::WorktreeListPorcelain => vec!["worktree", "list", "--porcelain"],
            Self::StatusPorcelainV2BranchZ => vec![
                "status",
                "--porcelain=v2",
                "-z",
                "--branch",
                "--untracked-files=all",
                "--ignored=matching",
            ],
            Self::VerifyCommit(_) => vec!["cat-file", "-e"],
            Self::VerifyCommits(_) => vec!["cat-file", "--batch-check=%(objectname) %(objecttype)"],
            Self::CheckFilterAttributes { .. } => {
                vec!["check-attr", "-z", "--stdin", "filter"]
            }
            Self::TreeEntries(_) => vec!["ls-tree", "--full-tree", "-r", "-l", "-z"],
            Self::ReadBlob(_) => vec!["cat-file", "blob"],
            Self::SnapshotBlob(_) => vec!["cat-file", "blob"],
            Self::AddWorktree { .. } => vec!["worktree", "add", "--detach", "--"],
            Self::AddWorktreeNoCheckout { .. } => {
                vec!["worktree", "add", "--detach", "--no-checkout", "--"]
            }
            Self::RemoveWorktree { .. } => vec!["worktree", "remove", "--"],
        };
        let mut arguments: Vec<OsString> = strings.into_iter().map(OsString::from).collect();
        match self {
            Self::VerifyCommit(object) => {
                arguments.push(format!("{}^{{commit}}", object.as_str()).into());
            }
            Self::LogSince { base, limit } => {
                arguments.push(format!("--max-count={limit}").into());
                arguments.push(format!("{}..HEAD", base.as_str()).into());
            }
            Self::DiffPaths { base, paths } => {
                arguments.push(base.as_str().into());
                arguments.push("--".into());
                for path in paths {
                    arguments.push(path.as_str().into());
                }
            }
            Self::CountSince { base } => {
                arguments.push(format!("{}..HEAD", base.as_str()).into());
            }
            Self::VerifyCommits(_) => {}
            Self::CheckIgnored(path) => arguments.push(path.into()),
            Self::CheckFilterAttributes {
                source: Some(source),
                ..
            } => {
                arguments.insert(1, format!("--source={}", source.as_str()).into());
            }
            Self::TreeEntries(object)
            | Self::ReadBlob(object)
            | Self::SnapshotBlob(object)
            | Self::InitializeIndex(object) => {
                arguments.push(object.as_str().into());
            }
            Self::AddWorktree { path, base } | Self::AddWorktreeNoCheckout { path, base } => {
                arguments.push(path.as_str().into());
                arguments.push(base.as_str().into());
            }
            Self::RemoveWorktree { path } => arguments.push(path.as_str().into()),
            _ => {}
        }
        arguments
    }

    pub(super) fn stdin_bytes(self) -> Result<Option<Vec<u8>>, GitError> {
        match self {
            Self::VerifyCommits(objects) => {
                let capacity = objects.iter().try_fold(0usize, |total, object| {
                    total
                        .checked_add(object.as_str().len() + 1)
                        .filter(|size| *size <= MAX_GIT_STDIN_BYTES)
                        .ok_or(GitError::OutputLimitExceeded)
                })?;
                let mut input = Vec::with_capacity(capacity);
                for object in objects {
                    input.extend_from_slice(object.as_str().as_bytes());
                    input.push(b'\n');
                }
                Ok(Some(input))
            }
            Self::CheckFilterAttributes { paths, .. } => {
                let capacity = paths.iter().try_fold(0usize, |total, path| {
                    if path.is_empty() || path.as_bytes().contains(&0) {
                        return Err(GitError::MalformedOutput);
                    }
                    total
                        .checked_add(path.len() + 1)
                        .filter(|size| *size <= MAX_GIT_STDIN_BYTES)
                        .ok_or(GitError::OutputLimitExceeded)
                })?;
                let mut input = Vec::with_capacity(capacity);
                for path in paths {
                    input.extend_from_slice(path.as_bytes());
                    input.push(0);
                }
                Ok(Some(input))
            }
            _ => Ok(None),
        }
    }
}
