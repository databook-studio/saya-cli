//! The working tree's changed paths, read for the edit-time shell-change
//! baseline.
//!
//! This path never opens the repository through [`super::repository_check`]:
//! an in-place root is a plain folder that may sit inside a working tree, and
//! the job only needs its changed paths. It reads the local config first (a
//! folder outside a repository answers `NotRepository` there), refuses a
//! config with filters or includes, and then reads porcelain-v2 status with
//! optional locks disabled so the index lock is never taken.

use std::path::{Path, PathBuf};
use std::time::Duration;

use saya_agent::CancellationToken;
use saya_types::GitObjectId;
use tokio::time::Instant;

use super::{
    GitError, GitOperation, GitService,
    command::require_success,
    parser::{InventoryBudget, repository_path},
    repository::GitInventoryError,
    request::GitRequest,
    status_fields::fixed_fields,
};

/// The most changed paths one working-tree read keeps. A folder past this is
/// not one a review can attribute shell changes in.
const MAX_WORKING_STATUS_PATHS: usize = 4096;

/// How a changed path appears in the working tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusKind {
    Added,
    Modified,
    Deleted,
}

/// One changed path, relative to the root the status was read in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitStatusEntry {
    pub path: PathBuf,
    pub kind: StatusKind,
}

impl GitService {
    /// The changed paths under `root`, root-relative and sorted by Git. A
    /// folder that is not inside a working tree answers
    /// [`GitError::NotRepository`]; a local config with filters or includes
    /// answers [`GitError::UnsupportedConfiguration`]; a listing past its
    /// bound answers [`GitError::RepositoryBoundExceeded`].
    pub async fn working_tree_status(
        &self,
        root: &Path,
        cancellation: &CancellationToken,
        deadline: Duration,
    ) -> Result<Vec<GitStatusEntry>, GitError> {
        let deadline_at = Instant::now() + deadline;
        let _guard = self
            .operation_guard_until(cancellation, deadline_at)
            .await?;
        self.ensure_supported_until(cancellation, deadline_at)
            .await?;
        // Outside a repository `git config --local` exits nonzero rather than
        // answering; that is the one failure this predicate reads as "not a
        // repository", so a plain folder falls back to an unrecorded baseline.
        let config = match self
            .read_until(root, GitRequest::LocalConfig, cancellation, deadline_at)
            .await
        {
            Ok(output) => output,
            Err(GitError::CommandFailed { .. }) => return Err(GitError::NotRepository),
            Err(error) => return Err(error),
        };
        if let Some(risk) = super::repository_check::config_risk(&config.stdout)? {
            return Err(GitError::UnsupportedConfiguration(risk));
        }
        let top = self
            .read_until(root, GitRequest::RepositoryRoot, cancellation, deadline_at)
            .await?;
        let top = canonical_dir(&top.stdout)?;
        let root = std::fs::canonicalize(root).map_err(|_| GitError::NotRepository)?;
        let prefix = root
            .strip_prefix(&top)
            .map_err(|_| GitError::NotRepository)?;
        let status = self
            .read_until(
                root.as_path(),
                GitRequest::StatusChangedZ,
                cancellation,
                deadline_at,
            )
            .await?;
        let mut budget = InventoryBudget::with_path_limit(MAX_WORKING_STATUS_PATHS);
        budget.add_output(&status.stdout).map_err(map_inventory)?;
        let entries = parse_status(&status.stdout, &mut budget).map_err(map_inventory)?;
        Ok(entries
            .into_iter()
            .filter_map(|entry| {
                let path = entry.path.strip_prefix(prefix).ok()?;
                Some(GitStatusEntry {
                    path: path.to_path_buf(),
                    kind: entry.kind,
                })
            })
            .collect())
    }

    /// HEAD's commit oid, or `None` when the branch is unborn.
    pub(crate) async fn head_oid(
        &self,
        root: &Path,
        cancellation: &CancellationToken,
        deadline: Duration,
    ) -> Result<Option<GitObjectId>, GitError> {
        let deadline_at = Instant::now() + deadline;
        let _guard = self
            .operation_guard_until(cancellation, deadline_at)
            .await?;
        let output = self
            .run_request_until(root, GitRequest::Head, cancellation, deadline_at)
            .await?;
        match require_success(output, GitOperation::Read) {
            Ok(output) => {
                let line = super::repository_check::single_line(&output.stdout)?;
                GitObjectId::parse(&line)
                    .map(Some)
                    .map_err(|_| GitError::MalformedOutput)
            }
            Err(GitError::CommandFailed { .. }) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// How many commits `base..HEAD` holds.
    pub(crate) async fn count_since(
        &self,
        root: &Path,
        base: &GitObjectId,
        cancellation: &CancellationToken,
        deadline: Duration,
    ) -> Result<u32, GitError> {
        let deadline_at = Instant::now() + deadline;
        let _guard = self
            .operation_guard_until(cancellation, deadline_at)
            .await?;
        let output = self
            .read_until(
                root,
                GitRequest::CountSince { base },
                cancellation,
                deadline_at,
            )
            .await?;
        super::repository_check::single_line(&output.stdout)?
            .parse::<u32>()
            .map_err(|_| GitError::MalformedOutput)
    }

    /// The raw diff of at most `paths` against `base`, bounded by the
    /// request's own 64 KiB capture limit.
    pub(crate) async fn diff_paths(
        &self,
        root: &Path,
        base: &GitObjectId,
        paths: &[String],
        cancellation: &CancellationToken,
        deadline: Duration,
    ) -> Result<Vec<u8>, GitError> {
        let deadline_at = Instant::now() + deadline;
        let _guard = self
            .operation_guard_until(cancellation, deadline_at)
            .await?;
        let output = self
            .read_until(
                root,
                GitRequest::DiffPaths { base, paths },
                cancellation,
                deadline_at,
            )
            .await?;
        Ok(output.stdout)
    }

    async fn read_until(
        &self,
        root: &Path,
        request: GitRequest<'_>,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<super::process::GitOutput, GitError> {
        let output = self
            .run_request_until(root, request, cancellation, deadline)
            .await?;
        require_success(output, GitOperation::Read)
    }
}

/// Parses porcelain-v2 `-z` records without headers: `1`, `2`, `u` and `?`
/// records name a changed path; `!` and `#` records are skipped. A `2`
/// record's following original-path record is consumed and not classified.
fn parse_status(
    bytes: &[u8],
    budget: &mut InventoryBudget,
) -> Result<Vec<GitStatusEntry>, GitInventoryError> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    if !bytes.ends_with(b"\0") {
        return Err(GitInventoryError::MalformedOutput);
    }
    let mut entries = Vec::new();
    let mut records = bytes[..bytes.len() - 1].split(|byte| *byte == 0);
    while let Some(record) = records.next() {
        if record.is_empty() {
            return Err(GitInventoryError::MalformedOutput);
        }
        match record.first() {
            Some(b'1') => {
                let (fields, path) = fixed_fields(record, 8)?;
                entries.push(changed(&fields, path, budget)?);
            }
            Some(b'2') => {
                let (fields, path) = fixed_fields(record, 9)?;
                entries.push(changed(&fields, path, budget)?);
                let original = records
                    .next()
                    .filter(|path| !path.is_empty())
                    .ok_or(GitInventoryError::MalformedOutput)?;
                let _ = repository_path(original)?;
            }
            Some(b'u') => {
                let (fields, path) = fixed_fields(record, 10)?;
                entries.push(changed(&fields, path, budget)?);
            }
            Some(b'?') => {
                let path_bytes = record
                    .strip_prefix(b"? ")
                    .filter(|path| !path.is_empty())
                    .ok_or(GitInventoryError::MalformedOutput)?;
                let path = repository_path(path_bytes.strip_suffix(b"/").unwrap_or(path_bytes))?;
                budget.add_path(path.as_os_str().as_encoded_bytes())?;
                entries.push(GitStatusEntry {
                    path,
                    kind: StatusKind::Added,
                });
            }
            Some(b'!') | Some(b'#') => {}
            _ => return Err(GitInventoryError::MalformedOutput),
        }
    }
    Ok(entries)
}

fn changed(
    fields: &[&[u8]],
    path: &[u8],
    budget: &mut InventoryBudget,
) -> Result<GitStatusEntry, GitInventoryError> {
    let xy = fields.get(1).ok_or(GitInventoryError::MalformedOutput)?;
    if xy.len() != 2 {
        return Err(GitInventoryError::MalformedOutput);
    }
    let path = repository_path(path)?;
    budget.add_path(path.as_os_str().as_encoded_bytes())?;
    let kind = if xy.contains(&b'A') {
        StatusKind::Added
    } else if xy.contains(&b'D') {
        StatusKind::Deleted
    } else {
        StatusKind::Modified
    };
    Ok(GitStatusEntry { path, kind })
}

fn canonical_dir(bytes: &[u8]) -> Result<PathBuf, GitError> {
    let path = PathBuf::from(super::repository_check::single_line(bytes)?);
    std::fs::canonicalize(path).map_err(|_| GitError::MalformedOutput)
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
