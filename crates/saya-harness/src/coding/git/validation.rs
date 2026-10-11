use std::{fs, path::Path};

use saya_agent::CancellationToken;

use super::{
    command::{GitError, GitOperation, GitRepository, require_success},
    inventory_deadline::InventoryDeadline,
    parser::InventoryBudget,
    repository::{GitBranchState, GitInventoryError},
    request::GitRequest,
    service::GitService,
};

pub(super) fn status_branch(value: &str) -> GitBranchState {
    match value {
        "(detached)" => GitBranchState::Detached,
        name => GitBranchState::Attached(name.to_owned()),
    }
}

pub(super) async fn read_line(
    service: &GitService,
    root: &Path,
    request: GitRequest<'_>,
    cancellation: &CancellationToken,
    budget: &mut InventoryBudget,
    deadline: InventoryDeadline,
) -> Result<String, GitInventoryError> {
    let output = service
        .run_request_until(root, request, cancellation, deadline.at())
        .await
        .map_err(GitInventoryError::Git)?;
    budget.add_output(&output.stdout)?;
    deadline.check()?;
    let output = require_success(output, GitOperation::Read).map_err(GitInventoryError::Git)?;
    super::repository_check::single_line(&output.stdout).map_err(GitInventoryError::Git)
}

pub(super) async fn verify_linked_worktree(
    service: &GitService,
    path: &Path,
    common_dir: &Path,
    cancellation: &CancellationToken,
    budget: &mut InventoryBudget,
    deadline: InventoryDeadline,
) -> Result<(), GitInventoryError> {
    deadline.check()?;
    let output = service
        .run_request_until(path, GitRequest::GitDir, cancellation, deadline.at())
        .await
        .map_err(|error| match error {
            GitError::Cancelled | GitError::TimedOut => GitInventoryError::Git(error),
            _ => GitInventoryError::UnsupportedWorktree,
        })?;
    budget.add_output(&output.stdout)?;
    deadline.check()?;
    let output = require_success(output, GitOperation::Read)
        .map_err(|_| GitInventoryError::UnsupportedWorktree)?;
    let path = super::repository_check::single_line(&output.stdout)
        .map_err(|_| GitInventoryError::UnsupportedWorktree)?;
    let git_dir = fs::canonicalize(path).map_err(|_| GitInventoryError::UnsupportedWorktree)?;
    let registered_parent = common_dir.join("worktrees");
    if !fs::metadata(&git_dir).is_ok_and(|metadata| metadata.is_dir())
        || git_dir.parent() != Some(registered_parent.as_path())
    {
        return Err(GitInventoryError::UnsupportedWorktree);
    }
    Ok(())
}

pub(super) async fn verify_heads(
    service: &GitService,
    repository: &GitRepository,
    heads: &[saya_types::GitObjectId],
    cancellation: &CancellationToken,
    budget: &mut InventoryBudget,
    deadline: InventoryDeadline,
) -> Result<(), GitInventoryError> {
    deadline.check()?;
    if heads.is_empty() {
        return Ok(());
    }
    let output = service
        .run_request_until(
            repository.root(),
            GitRequest::VerifyCommits(heads),
            cancellation,
            deadline.at(),
        )
        .await
        .map_err(GitInventoryError::Git)?;
    budget.add_output(&output.stdout)?;
    let output = require_success(output, GitOperation::Read).map_err(GitInventoryError::Git)?;
    let lines: Vec<_> = output.stdout.split(|byte| *byte == b'\n').collect();
    if lines.len() != heads.len() + 1 || lines.last() != Some(&&b""[..]) {
        return Err(GitInventoryError::MalformedOutput);
    }
    for (line, expected) in lines.iter().take(heads.len()).zip(heads) {
        deadline.check()?;
        let mut fields = line.split(|byte| *byte == b' ');
        let object = fields.next().ok_or(GitInventoryError::MalformedOutput)?;
        if fields.next() != Some(&b"commit"[..]) || fields.next().is_some() {
            return Err(GitInventoryError::UnsupportedWorktree);
        }
        if object != expected.as_str().as_bytes() {
            return Err(GitInventoryError::MalformedOutput);
        }
    }
    deadline.check()?;
    Ok(())
}
