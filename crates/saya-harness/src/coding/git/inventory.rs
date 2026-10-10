use std::{collections::HashSet, fs, path::PathBuf};

use saya_agent::CancellationToken;

use super::{
    command::{GitError, GitRepository, require_success},
    index_parser::{reject_sparse_index, validate_index_entries},
    inventory_deadline::InventoryDeadline,
    inventory_types::GitRepositoryInventory,
    metadata_scan::reject_nested_repositories,
    parser::{InventoryBudget, MAX_INVENTORY_PATHS},
    repository::{
        GitBranchState, GitHeadState, GitInventoryError, GitRepositoryIdentity, identity,
    },
    request::GitRequest,
    service::GitService,
    status_parser::parse_status,
    validation::{read_line, status_branch, verify_heads, verify_linked_worktree},
    worktree_parser::parse_worktrees,
};

impl GitService {
    /// Read bounded repository and worktree metadata without loading file contents.
    pub async fn inventory_repository(
        &self,
        repository: &GitRepository,
        cancellation: &CancellationToken,
    ) -> Result<GitRepositoryInventory, GitInventoryError> {
        let deadline = InventoryDeadline::start();
        let _guard = tokio::select! {
            biased;
            _ = cancellation.cancelled() => {
                return Err(GitInventoryError::Git(GitError::Cancelled));
            }
            result = tokio::time::timeout_at(deadline.at(), self.operation_guard()) => {
                match result {
                    Ok(guard) => guard,
                    Err(_) => return Err(GitInventoryError::Git(GitError::TimedOut)),
                }
            }
        };
        deadline.check()?;
        super::repository_check::verify_with_deadline(
            self,
            repository,
            cancellation,
            Some(deadline.at()),
        )
        .await
        .map_err(GitInventoryError::Git)?;
        deadline.check()?;
        self.ensure_supported_until(cancellation, deadline.at())
            .await
            .map_err(GitInventoryError::Git)?;
        let mut budget = InventoryBudget::default();

        let git_dir = read_line(
            self,
            repository.root(),
            GitRequest::GitDir,
            cancellation,
            &mut budget,
            deadline,
        )
        .await?;
        let git_dir = PathBuf::from(git_dir);
        let git_dir =
            fs::canonicalize(git_dir).map_err(|_| GitInventoryError::UnsupportedRepositoryState)?;
        if !fs::metadata(&git_dir).is_ok_and(|metadata| metadata.is_dir())
            || !git_dir.starts_with(repository.common_dir())
        {
            return Err(GitInventoryError::UnsupportedRepositoryState);
        }
        let (repository_id, common_dir_sha256) = identity(repository.common_dir())?;
        let identity = GitRepositoryIdentity {
            repository_id,
            common_dir_sha256,
            root: repository.root().to_path_buf(),
            git_dir,
            common_dir: repository.common_dir().to_path_buf(),
            object_format: repository.object_format(),
        };

        let worktree_output = self
            .run_request_until(
                repository.root(),
                GitRequest::WorktreeListPorcelainZ,
                cancellation,
                deadline.at(),
            )
            .await
            .map_err(GitInventoryError::Git)?;
        let worktree_output = require_success(worktree_output, super::command::GitOperation::Read)
            .map_err(GitInventoryError::Git)?;
        budget.add_output(&worktree_output.stdout)?;
        let records = parse_worktrees(&worktree_output.stdout, repository.object_format())?;
        deadline.check()?;
        if records.len() > MAX_INVENTORY_PATHS {
            return Err(GitInventoryError::TooLarge);
        }
        let mut seen_worktrees = HashSet::new();
        let mut worktrees = Vec::with_capacity(records.len());
        let mut current = None;
        let mut heads = Vec::new();
        for record in records {
            deadline.check()?;
            let raw_path = record.path.clone();
            let canonical =
                fs::canonicalize(&raw_path).map_err(|_| GitInventoryError::UnsupportedWorktree)?;
            if canonical != raw_path
                || !fs::metadata(&canonical).is_ok_and(|metadata| metadata.is_dir())
            {
                return Err(GitInventoryError::UnsupportedWorktree);
            }
            let encoded = canonical.as_os_str().as_encoded_bytes();
            budget.add_path(encoded)?;
            if !seen_worktrees.insert(encoded.to_vec()) {
                return Err(GitInventoryError::MalformedOutput);
            }
            let is_current = canonical == repository.root();
            if is_current {
                if current.is_some() {
                    return Err(GitInventoryError::MalformedOutput);
                }
                current = Some((record.head.clone(), record.branch.clone()));
            } else {
                verify_linked_worktree(
                    self,
                    &canonical,
                    repository.common_dir(),
                    cancellation,
                    &mut budget,
                    deadline,
                )
                .await?;
            }
            if let GitHeadState::Commit(head) = &record.head {
                heads.push(head.clone());
            }
            worktrees.push(record.into_inventory(canonical));
        }
        let (head, branch) = current.ok_or(GitInventoryError::UnsupportedWorktree)?;

        let status_output = self
            .run_request_until(
                repository.root(),
                GitRequest::StatusPorcelainV2BranchZ,
                cancellation,
                deadline.at(),
            )
            .await
            .map_err(GitInventoryError::Git)?;
        let status_output = require_success(status_output, super::command::GitOperation::Read)
            .map_err(GitInventoryError::Git)?;
        budget.add_output(&status_output.stdout)?;
        deadline.check()?;
        let status = parse_status(
            &status_output.stdout,
            repository.object_format(),
            &mut budget,
        )?;
        deadline.check()?;
        let parsed_branch = match &status.head {
            GitHeadState::Unborn => GitBranchState::Unborn(status.branch.clone()),
            GitHeadState::Commit(_) => status_branch(&status.branch),
        };
        if status.head != head || parsed_branch != branch {
            return Err(GitInventoryError::MalformedOutput);
        }

        let stage_output = self
            .run_request_until(
                repository.root(),
                GitRequest::IndexStageEntries,
                cancellation,
                deadline.at(),
            )
            .await
            .map_err(GitInventoryError::Git)?;
        let stage_output = require_success(stage_output, super::command::GitOperation::Read)
            .map_err(GitInventoryError::Git)?;
        budget.add_output(&stage_output.stdout)?;
        deadline.check()?;
        let indexed_paths = validate_index_entries(
            &stage_output.stdout,
            repository.object_format(),
            &mut budget,
        )?;
        deadline.check()?;
        let index_output = self
            .run_request_until(
                repository.root(),
                GitRequest::IndexEntries,
                cancellation,
                deadline.at(),
            )
            .await
            .map_err(GitInventoryError::Git)?;
        let index_output = require_success(index_output, super::command::GitOperation::Read)
            .map_err(GitInventoryError::Git)?;
        budget.add_output(&index_output.stdout)?;
        deadline.check()?;
        reject_sparse_index(&index_output.stdout)?;
        deadline.check()?;
        reject_nested_repositories(
            repository.root(),
            &status.paths,
            &indexed_paths,
            deadline,
            cancellation,
        )?;
        deadline.check()?;

        verify_heads(
            self,
            repository,
            &heads,
            cancellation,
            &mut budget,
            deadline,
        )
        .await?;
        if let (
            GitHeadState::Commit(expected),
            GitBranchState::Attached(_) | GitBranchState::Detached,
        ) = (&head, &branch)
        {
            let actual = read_line(
                self,
                repository.root(),
                GitRequest::Head,
                cancellation,
                &mut budget,
                deadline,
            )
            .await?;
            let actual = saya_types::GitObjectId::parse(&actual)
                .map_err(|_| GitInventoryError::MalformedOutput)?;
            if actual != *expected {
                return Err(GitInventoryError::MalformedOutput);
            }
        }
        deadline.check()?;

        Ok(GitRepositoryInventory {
            identity,
            head,
            branch,
            paths: status.paths,
            worktrees,
        })
    }
}
