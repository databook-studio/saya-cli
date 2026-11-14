use super::{
    GitError, GitOperation, GitService,
    command::require_success,
    parser::{InventoryBudget, repository_path},
    repository::GitInventoryError,
    request::GitRequest,
};
use saya_agent::CancellationToken;
use saya_types::{ManagedCheckoutId, WorkspacePath};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
use tokio::time::{Duration, Instant};

/// The checkout's own Git ignore verdicts, as directory markers and files.
///
/// The listing is bounded and produced by the shared Git service with its
/// neutralised environment; repository-defined filters and text conversions
/// never run. A directory marker covers its whole subtree, so an ignored
/// build tree is one entry regardless of how many files it holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IgnoredPaths {
    directories: BTreeSet<PathBuf>,
    files: BTreeSet<PathBuf>,
    total: usize,
}

impl IgnoredPaths {
    /// No ignore facts: every path is a candidate. Used by the in-place path
    /// and by callers whose tree has no repository ignore rules.
    pub fn none() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.total == 0
    }

    pub fn len(&self) -> usize {
        self.total
    }

    /// Whether `path` is ignored, either directly or because an ancestor
    /// directory is ignored.
    pub fn covers(&self, path: &Path) -> bool {
        if self.files.contains(path) || self.directories.contains(path) {
            return true;
        }
        path.ancestors()
            .skip(1)
            .any(|ancestor| !ancestor.as_os_str().is_empty() && self.directories.contains(ancestor))
    }

    fn insert(&mut self, path: PathBuf, directory: bool) {
        self.total = self.total.saturating_add(1);
        if directory {
            self.directories.insert(path);
        } else {
            self.files.insert(path);
        }
    }
}

impl GitService {
    /// Reads one current ignore verdict in an already-verified owned checkout.
    /// The service selects its root; model data supplies only a bounded path.
    pub async fn owned_path_is_ignored(
        &self,
        checkout: &ManagedCheckoutId,
        path: &WorkspacePath,
        cancellation: &CancellationToken,
    ) -> Result<bool, GitError> {
        if path.as_str().len() > 512 {
            return Err(GitError::MalformedOutput);
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        let _guard = self.operation_guard_until(cancellation, deadline).await?;
        let root = self.managed_path(checkout)?;
        let output = self
            .run_request_until(
                Path::new(root.as_str()),
                GitRequest::CheckIgnored(path.as_str()),
                cancellation,
                deadline,
            )
            .await?;
        match (output.exit_code, output.signal) {
            (Some(0), None) => Ok(true),
            (Some(1), None) => Ok(false),
            _ => Err(GitError::CommandFailed {
                operation: GitOperation::Read,
                exit_code: output.exit_code,
                detail: super::command::failure_detail(&output.stderr),
            }),
        }
    }

    /// Reads the checkout's ignored directories and files, bounded by the
    /// shared Git output and path budgets. `--untracked-files=normal` reports
    /// an ignored directory as one marker, so the cost follows the number of
    /// ignore rules, not the number of generated files.
    pub async fn ignored_paths(
        &self,
        root: &Path,
        cancellation: &CancellationToken,
        deadline: Duration,
    ) -> Result<IgnoredPaths, GitError> {
        let deadline_at = Instant::now() + deadline;
        let _guard = self
            .operation_guard_until(cancellation, deadline_at)
            .await?;
        let output = self
            .run_request_until(root, GitRequest::IgnoredPaths, cancellation, deadline_at)
            .await?;
        let output = require_success(output, GitOperation::Read)?;
        let mut budget = InventoryBudget::default();
        budget.add_output(&output.stdout).map_err(map_inventory)?;
        parse_ignored_paths(&output.stdout, &mut budget).map_err(map_inventory)
    }
}

/// Parses the `! ` records of porcelain-v2 status output. Every other record
/// (untracked, changed, headers) is not an ignore verdict and is skipped.
fn parse_ignored_paths(
    bytes: &[u8],
    budget: &mut InventoryBudget,
) -> Result<IgnoredPaths, GitInventoryError> {
    if bytes.is_empty() {
        return Ok(IgnoredPaths::default());
    }
    if !bytes.ends_with(b"\0") {
        return Err(GitInventoryError::MalformedOutput);
    }
    let mut ignored = IgnoredPaths::default();
    for record in bytes[..bytes.len() - 1].split(|byte| *byte == 0) {
        let Some(rest) = record.strip_prefix(b"! ") else {
            continue;
        };
        let directory = rest.ends_with(b"/");
        let path_bytes = rest.strip_suffix(b"/").unwrap_or(rest);
        let path = repository_path(path_bytes)?;
        budget.add_path(path.as_os_str().as_encoded_bytes())?;
        ignored.insert(path, directory);
    }
    Ok(ignored)
}

fn map_inventory(error: GitInventoryError) -> GitError {
    match error {
        GitInventoryError::Git(error) => error,
        GitInventoryError::BoundExceeded {
            bound,
            measured,
            limit,
        } => GitError::RepositoryBoundExceeded {
            bound,
            measured,
            limit,
        },
        _ => GitError::MalformedOutput,
    }
}
