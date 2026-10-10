use std::ffi::OsString;

use saya_types::{GitObjectId, ManagedCheckoutPath};

use super::command::GitOperation;

const MAX_GIT_STDOUT_BYTES: usize = 8 * 1024 * 1024;
const MAX_GIT_STDERR_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy)]
pub(super) enum GitRequest<'a> {
    Version,
    IsBare,
    RepositoryRoot,
    GitDir,
    CommonDir,
    ObjectFormat,
    LocalConfig,
    VerifyCommit(&'a GitObjectId),
    VerifyCommits(&'a [GitObjectId]),
    IndexEntries,
    IndexStageEntries,
    TreeEntries(&'a GitObjectId),
    ReadBlob(&'a GitObjectId),
    Head,
    Branch,
    UnstagedNames,
    StagedNames,
    Status,
    WorktreeListPorcelainZ,
    StatusPorcelainV2BranchZ,
    AddWorktree {
        path: &'a ManagedCheckoutPath,
        base: &'a GitObjectId,
    },
    RemoveWorktree {
        path: &'a ManagedCheckoutPath,
    },
}

impl GitRequest<'_> {
    pub(super) fn operation(self) -> GitOperation {
        match self {
            Self::AddWorktree { .. } => GitOperation::Materialize,
            Self::RemoveWorktree { .. } => GitOperation::Remove,
            _ => GitOperation::Read,
        }
    }

    pub(super) fn output_limits(self) -> (usize, usize) {
        match self {
            Self::Version => (4096, 64 * 1024),
            Self::LocalConfig | Self::ReadBlob(_) => (1024 * 1024, 64 * 1024),
            Self::AddWorktree { .. } | Self::RemoveWorktree { .. } => (1024 * 1024, 1024 * 1024),
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
            Self::IndexEntries => vec!["ls-files", "-v", "-z"],
            Self::IndexStageEntries => vec!["ls-files", "--stage", "-z"],
            Self::Head => vec!["rev-parse", "--verify", "HEAD^{commit}"],
            Self::Branch => vec!["rev-parse", "--abbrev-ref", "HEAD"],
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
            Self::WorktreeListPorcelainZ => vec!["worktree", "list", "--porcelain", "-z"],
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
            Self::TreeEntries(_) => vec!["ls-tree", "--full-tree", "-r", "-l", "-z"],
            Self::ReadBlob(_) => vec!["cat-file", "blob"],
            Self::AddWorktree { .. } => vec!["worktree", "add", "--detach", "--"],
            Self::RemoveWorktree { .. } => vec!["worktree", "remove", "--"],
        };
        let mut arguments: Vec<OsString> = strings.into_iter().map(OsString::from).collect();
        match self {
            Self::VerifyCommit(object) => {
                arguments.push(format!("{}^{{commit}}", object.as_str()).into());
            }
            Self::VerifyCommits(_) => {}
            Self::TreeEntries(object) | Self::ReadBlob(object) => {
                arguments.push(object.as_str().into());
            }
            Self::AddWorktree { path, base } => {
                arguments.push(path.as_str().into());
                arguments.push(base.as_str().into());
            }
            Self::RemoveWorktree { path } => arguments.push(path.as_str().into()),
            _ => {}
        }
        arguments
    }

    pub(super) fn stdin_bytes(self) -> Option<Vec<u8>> {
        let Self::VerifyCommits(objects) = self else {
            return None;
        };
        let capacity = objects.iter().map(|object| object.as_str().len() + 1).sum();
        let mut input = Vec::with_capacity(capacity);
        for object in objects {
            input.extend_from_slice(object.as_str().as_bytes());
            input.push(b'\n');
        }
        Some(input)
    }
}
