use std::{fs, io, path::Path};

use saya_agent::CancellationToken;
use saya_types::{GitObjectId, ManagedCheckoutId};

use super::{
    command::{GitError, GitOperation, GitRepository, ManagedWorktree, require_success, unknown},
    repository_check,
    request::GitRequest,
    service::GitService,
};

impl GitService {
    pub async fn add_worktree(
        &self,
        repository: &GitRepository,
        base_commit: &GitObjectId,
        checkout_id: &ManagedCheckoutId,
        cancellation: &CancellationToken,
    ) -> Result<ManagedWorktree, GitError> {
        self.add_worktree_inner(repository, base_commit, checkout_id, cancellation, false)
            .await
    }

    pub async fn add_worktree_no_checkout(
        &self,
        repository: &GitRepository,
        base_commit: &GitObjectId,
        checkout_id: &ManagedCheckoutId,
        cancellation: &CancellationToken,
    ) -> Result<ManagedWorktree, GitError> {
        self.add_worktree_inner(repository, base_commit, checkout_id, cancellation, true)
            .await
    }

    pub async fn adopt_worktree(
        &self,
        repository: &GitRepository,
        base_commit: &GitObjectId,
        checkout_id: &ManagedCheckoutId,
        cancellation: &CancellationToken,
    ) -> Result<ManagedWorktree, GitError> {
        let _guard = self.operation_guard().await;
        self.ensure_supported(cancellation).await?;
        repository_check::verify(self, repository, cancellation).await?;
        let managed_path = self.managed_path(checkout_id)?;
        let path = Path::new(managed_path.as_str());
        self.verify_created_worktree(repository, path, base_commit, cancellation)
            .await?;
        Ok(ManagedWorktree {
            service_root: self.environment.root.clone(),
            repository: repository.clone(),
            checkout_id: checkout_id.clone(),
            path: managed_path,
            base_commit: base_commit.clone(),
        })
    }

    async fn add_worktree_inner(
        &self,
        repository: &GitRepository,
        base_commit: &GitObjectId,
        checkout_id: &ManagedCheckoutId,
        cancellation: &CancellationToken,
        no_checkout: bool,
    ) -> Result<ManagedWorktree, GitError> {
        let _guard = self.operation_guard().await;
        self.ensure_supported(cancellation).await?;
        repository_check::verify(self, repository, cancellation).await?;
        if !repository_check::object_id_matches(repository.object_format, base_commit) {
            return Err(GitError::InvalidBaseCommit);
        }
        super::materialization_check::validate_materialization(
            self,
            repository,
            base_commit,
            cancellation,
        )
        .await?;
        let commit = self
            .run_request(
                &repository.root,
                GitRequest::VerifyCommit(base_commit),
                cancellation,
            )
            .await?;
        require_success(commit, GitOperation::Read).map_err(|_| GitError::InvalidBaseCommit)?;

        let managed_path = self.managed_path(checkout_id)?;
        let path = Path::new(managed_path.as_str());
        match fs::symlink_metadata(path) {
            Ok(_) => return Err(GitError::ManagedPathAlreadyExists),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err(GitError::InvalidManagedPath),
        }
        if !path.starts_with(&self.environment.worktrees)
            || path.parent() != Some(self.environment.worktrees.as_path())
        {
            return Err(GitError::ManagedPathOutsideState);
        }
        let request = if no_checkout {
            GitRequest::AddWorktreeNoCheckout {
                path: &managed_path,
                base: base_commit,
            }
        } else {
            GitRequest::AddWorktree {
                path: &managed_path,
                base: base_commit,
            }
        };
        let created = self
            .run_request(&repository.root, request, cancellation)
            .await
            .and_then(|output| require_success(output, GitOperation::Materialize))
            .map_err(|source| unknown(GitOperation::Materialize, source))?;
        drop(created);

        self.verify_created_worktree(repository, path, base_commit, cancellation)
            .await?;
        Ok(ManagedWorktree {
            service_root: self.environment.root.clone(),
            repository: repository.clone(),
            checkout_id: checkout_id.clone(),
            path: managed_path,
            base_commit: base_commit.clone(),
        })
    }

    async fn verify_created_worktree(
        &self,
        repository: &GitRepository,
        path: &Path,
        base_commit: &GitObjectId,
        cancellation: &CancellationToken,
    ) -> Result<(), GitError> {
        let identity = repository_check::inspect(self, path, true, cancellation)
            .await
            .map_err(|source| unknown(GitOperation::Materialize, source))?;
        let head = self
            .run_request(path, GitRequest::Head, cancellation)
            .await
            .and_then(|output| require_success(output, GitOperation::Read))
            .and_then(|output| repository_check::single_line(&output.stdout))
            .map_err(|source| unknown(GitOperation::Materialize, source))?;
        let branch = self
            .run_request(path, GitRequest::Branch, cancellation)
            .await
            .and_then(|output| require_success(output, GitOperation::Read))
            .and_then(|output| repository_check::single_line(&output.stdout))
            .map_err(|source| unknown(GitOperation::Materialize, source))?;
        if identity.root.as_path() != path
            || identity.common_dir != repository.common_dir
            || head != base_commit.as_str()
            || branch != "HEAD"
        {
            return Err(unknown(
                GitOperation::Materialize,
                GitError::RepositoryChanged,
            ));
        }
        Ok(())
    }

    pub async fn remove_worktree(
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
        let current = repository_check::inspect(self, path, true, cancellation).await?;
        if current.root.as_path() != path || current.common_dir != worktree.repository.common_dir {
            return Err(GitError::WorktreeOwnershipMismatch);
        }
        let head = self
            .run_request(path, GitRequest::Head, cancellation)
            .await
            .and_then(|output| require_success(output, GitOperation::Read))
            .and_then(|output| repository_check::single_line(&output.stdout))?;
        let branch = self
            .run_request(path, GitRequest::Branch, cancellation)
            .await
            .and_then(|output| require_success(output, GitOperation::Read))
            .and_then(|output| repository_check::single_line(&output.stdout))?;
        if head != worktree.base_commit.as_str() || branch != "HEAD" {
            return Err(GitError::WorktreeNotClean);
        }
        for request in [
            GitRequest::UnstagedNames,
            GitRequest::StagedNames,
            GitRequest::Status,
        ] {
            let output = self
                .run_request(path, request, cancellation)
                .await
                .and_then(|output| require_success(output, GitOperation::Read))?;
            if !output.stdout.is_empty() {
                return Err(GitError::WorktreeNotClean);
            }
        }
        self.run_request(
            &worktree.repository.root,
            GitRequest::RemoveWorktree {
                path: &worktree.path,
            },
            cancellation,
        )
        .await
        .and_then(|output| require_success(output, GitOperation::Remove))
        .map_err(|source| unknown(GitOperation::Remove, source))?;
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Ok(_) => {
                return Err(unknown(
                    GitOperation::Remove,
                    GitError::WorktreeOwnershipMismatch,
                ));
            }
            Err(_) => return Err(unknown(GitOperation::Remove, GitError::InvalidManagedPath)),
        }
        Ok(())
    }
}
