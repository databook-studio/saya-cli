//! Initialize only a still-Creating managed checkout's private index.
use super::{
    GitError, GitOperation, GitService, ManagedWorktree, command::require_success,
    repository_check, request::GitRequest,
};
use saya_agent::CancellationToken;
use std::{fs, path::Path};

impl GitService {
    /// Crate-only create orchestration calls this before handing out a checkout.
    /// Existing indexes must already match HEAD; no staged change is discarded.
    pub(crate) async fn initialize_materialization_index(
        &self,
        worktree: &ManagedWorktree,
        cancellation: &CancellationToken,
    ) -> Result<(), GitError> {
        let _guard = self.operation_guard().await;
        self.ensure_supported(cancellation).await?;
        if worktree.service_root != self.environment.root
            || self.managed_path(&worktree.checkout_id)? != worktree.path
        {
            return Err(GitError::WorktreeOwnershipMismatch);
        }
        repository_check::verify(self, &worktree.repository, cancellation).await?;
        let path = Path::new(worktree.path.as_str());
        let identity = repository_check::inspect(self, path, true, cancellation).await?;
        if identity.root != path || identity.common_dir != worktree.repository.common_dir {
            return Err(GitError::WorktreeOwnershipMismatch);
        }
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
