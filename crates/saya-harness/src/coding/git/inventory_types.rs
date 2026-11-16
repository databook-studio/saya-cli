use std::{
    fmt,
    path::{Path, PathBuf},
};

use super::repository::{GitBranchState, GitHeadState, GitRepositoryIdentity};

#[derive(Clone, PartialEq, Eq)]
pub struct GitWorktreeInventory {
    pub(super) path: PathBuf,
    pub(super) head: GitHeadState,
    pub(super) branch: GitBranchState,
    pub(super) locked: bool,
}

impl GitWorktreeInventory {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn head(&self) -> &GitHeadState {
        &self.head
    }

    pub fn branch(&self) -> &GitBranchState {
        &self.branch
    }

    pub fn locked(&self) -> bool {
        self.locked
    }
}

impl fmt::Debug for GitWorktreeInventory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GitWorktreeInventory")
            .field("path", &"[worktree path]")
            .field("head", &self.head)
            .field("branch", &self.branch)
            .field("locked", &self.locked)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct GitPathStatus {
    pub(super) path: PathBuf,
    pub(super) original_path: Option<PathBuf>,
    pub(super) staged: bool,
    pub(super) unstaged: bool,
    pub(super) staged_added: bool,
    pub(super) unstaged_added: bool,
    pub(super) staged_deleted: bool,
    pub(super) unstaged_deleted: bool,
    pub(super) untracked: bool,
    pub(super) untracked_directory: bool,
    pub(super) ignored: bool,
    pub(super) conflicted: bool,
}

impl GitPathStatus {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn original_path(&self) -> Option<&Path> {
        self.original_path.as_deref()
    }

    pub fn staged(&self) -> bool {
        self.staged
    }

    pub fn unstaged(&self) -> bool {
        self.unstaged
    }

    pub fn staged_added(&self) -> bool {
        self.staged_added
    }

    pub fn unstaged_added(&self) -> bool {
        self.unstaged_added
    }

    pub fn staged_deleted(&self) -> bool {
        self.staged_deleted
    }

    pub fn unstaged_deleted(&self) -> bool {
        self.unstaged_deleted
    }

    pub fn untracked(&self) -> bool {
        self.untracked
    }

    pub fn ignored(&self) -> bool {
        self.ignored
    }

    pub fn conflicted(&self) -> bool {
        self.conflicted
    }
}

impl fmt::Debug for GitPathStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GitPathStatus")
            .field("path", &"[repository path]")
            .field("has_original_path", &self.original_path.is_some())
            .field("staged", &self.staged)
            .field("unstaged", &self.unstaged)
            .field("staged_added", &self.staged_added)
            .field("unstaged_added", &self.unstaged_added)
            .field("staged_deleted", &self.staged_deleted)
            .field("unstaged_deleted", &self.unstaged_deleted)
            .field("untracked", &self.untracked)
            .field("untracked_directory", &self.untracked_directory)
            .field("ignored", &self.ignored)
            .field("conflicted", &self.conflicted)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct GitRepositoryInventory {
    pub(super) identity: GitRepositoryIdentity,
    pub(super) head: GitHeadState,
    pub(super) branch: GitBranchState,
    pub(super) paths: Vec<GitPathStatus>,
    pub(super) worktrees: Vec<GitWorktreeInventory>,
}

impl GitRepositoryInventory {
    pub fn identity(&self) -> &GitRepositoryIdentity {
        &self.identity
    }

    pub fn head(&self) -> &GitHeadState {
        &self.head
    }

    pub fn branch(&self) -> &GitBranchState {
        &self.branch
    }

    pub fn paths(&self) -> &[GitPathStatus] {
        &self.paths
    }

    pub fn worktrees(&self) -> &[GitWorktreeInventory] {
        &self.worktrees
    }
}

impl fmt::Debug for GitRepositoryInventory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GitRepositoryInventory")
            .field("identity", &self.identity)
            .field("head", &self.head)
            .field("branch", &self.branch)
            .field("path_count", &self.paths.len())
            .field("worktree_count", &self.worktrees.len())
            .finish()
    }
}
