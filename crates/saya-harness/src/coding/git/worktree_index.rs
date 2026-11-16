//! Initialize only a still-Creating managed checkout's private index.
use super::{
    GitError, GitOperation, GitService, ManagedWorktree, command::require_success,
    repository_check, request::GitRequest,
};
use saya_agent::CancellationToken;
use std::{fs, path::Path};

impl GitService {
    /// The create flow's own path: `worktree` is the value
    /// `add_worktree_no_checkout` or `adopt_worktree` returned in this same
    /// call, after they re-verified the repository and the created worktree.
    /// The create lock is held, the target is private, and nothing mutated it
    /// in between, so the repository check and the worktree identity
    /// inspection are not repeated; the head, worktree git dir, index and
    /// staged-name checks still run.
    pub(crate) async fn initialize_materialization_index_verified(
        &self,
        worktree: &ManagedWorktree,
        cancellation: &CancellationToken,
    ) -> Result<(), GitError> {
        let _guard = self.operation_guard().await;
        self.ensure_supported(cancellation).await?;
        self.check_managed_worktree_ownership(worktree)?;
        self.initialize_index_locked(worktree, cancellation).await
    }

    fn check_managed_worktree_ownership(&self, worktree: &ManagedWorktree) -> Result<(), GitError> {
        if worktree.service_root != self.environment.root
            || self.managed_path(&worktree.checkout_id)? != worktree.path
        {
            return Err(GitError::WorktreeOwnershipMismatch);
        }
        Ok(())
    }

    /// The index checks both entry points share: HEAD still names the base
    /// commit, the worktree's git dir is under this repository's `worktrees`,
    /// an existing index is a regular file (never a staged change), and the
    /// index is empty after initialization.
    async fn initialize_index_locked(
        &self,
        worktree: &ManagedWorktree,
        cancellation: &CancellationToken,
    ) -> Result<(), GitError> {
        let path = Path::new(worktree.path.as_str());
        let head = self
            .run_request(path, GitRequest::Head, cancellation)
            .await?;
        let head = require_success(head, GitOperation::Read)?;
        if repository_check::single_line(&head.stdout)? != worktree.base_commit.as_str() {
            return Err(GitError::WorktreeOwnershipMismatch);
        }
        let output = self
            .run_request(path, GitRequest::GitDir, cancellation)
            .await?;
        let output = require_success(output, GitOperation::Read)?;
        let git_dir = fs::canonicalize(repository_check::single_line(&output.stdout)?)
            .map_err(|_| GitError::InvalidManagedPath)?;
        let managed_git = worktree.repository.common_dir.join("worktrees");
        if git_dir.parent() != Some(managed_git.as_path()) {
            return Err(GitError::WorktreeOwnershipMismatch);
        }
        let index = git_dir.join("index");
        match fs::symlink_metadata(&index) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.run_request(
                    path,
                    GitRequest::InitializeIndex(&worktree.base_commit),
                    cancellation,
                )
                .await
                .and_then(|out| require_success(out, GitOperation::Materialize))?;
            }
            _ => return Err(GitError::WorktreeOwnershipMismatch),
        }
        let staged = self
            .run_request(path, GitRequest::StagedNames, cancellation)
            .await?;
        if !require_success(staged, GitOperation::Read)?
            .stdout
            .is_empty()
        {
            return Err(GitError::WorktreeNotClean);
        }
        Ok(())
    }
}
