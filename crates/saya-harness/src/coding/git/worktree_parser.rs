use std::path::PathBuf;

use saya_types::GitObjectId;

use super::{
    command::GitObjectFormat,
    inventory_types::GitWorktreeInventory,
    parser::{MAX_INVENTORY_PATHS, path_from_git_bytes},
    repository::{GitBranchState, GitHeadState, GitInventoryError},
    status_fields::parse_oid,
};

pub(super) struct WorktreeRecord {
    pub(super) path: PathBuf,
    pub(super) head: GitHeadState,
    pub(super) branch: GitBranchState,
    pub(super) locked: bool,
}

pub(super) fn parse_worktrees(
    bytes: &[u8],
    format: GitObjectFormat,
) -> Result<Vec<WorktreeRecord>, GitInventoryError> {
    if !bytes.ends_with(b"\0\0") {
        return Err(GitInventoryError::MalformedOutput);
    }
    let mut records = Vec::new();
    let mut current: Option<PartialWorktree> = None;
    for field in bytes[..bytes.len() - 1].split(|byte| *byte == 0) {
        if field.is_empty() {
            let record = current.take().ok_or(GitInventoryError::MalformedOutput)?;
            records.push(record.finish()?);
            if records.len() > MAX_INVENTORY_PATHS {
                return Err(GitInventoryError::TooLarge);
            }
            continue;
        }
        if let Some(path) = field.strip_prefix(b"worktree ") {
            if current.is_some() || path.is_empty() {
                return Err(GitInventoryError::MalformedOutput);
            }
            let path = path_from_git_bytes(path)?;
            if !path.is_absolute() {
                return Err(GitInventoryError::UnsafePath);
            }
            current = Some(PartialWorktree::new(path));
            continue;
        }
        let record = current.as_mut().ok_or(GitInventoryError::MalformedOutput)?;
        record.push_field(field, format)?;
    }
    if current.is_some() {
        return Err(GitInventoryError::MalformedOutput);
    }
    if records.is_empty() {
        return Err(GitInventoryError::MalformedOutput);
    }
    Ok(records)
}

struct PartialWorktree {
    path: PathBuf,
    head: Option<GitObjectId>,
    head_seen: bool,
    branch: Option<String>,
    detached: bool,
    locked: bool,
}

impl PartialWorktree {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            head: None,
            head_seen: false,
            branch: None,
            detached: false,
            locked: false,
        }
    }

    fn push_field(
        &mut self,
        field: &[u8],
        format: GitObjectFormat,
    ) -> Result<(), GitInventoryError> {
        if let Some(head) = field.strip_prefix(b"HEAD ") {
            if self.head_seen {
                return Err(GitInventoryError::MalformedOutput);
            }
            self.head_seen = true;
            let length = match format {
                GitObjectFormat::Sha1 => 40,
                GitObjectFormat::Sha256 => 64,
            };
            if head.len() != length {
                return Err(GitInventoryError::MalformedOutput);
            }
            if head.iter().any(|byte| *byte != b'0') {
                self.head = Some(parse_oid(head, format)?);
            }
        } else if let Some(branch) = field.strip_prefix(b"branch ") {
            if self.branch.is_some() || branch.is_empty() {
                return Err(GitInventoryError::MalformedOutput);
            }
            let branch =
                std::str::from_utf8(branch).map_err(|_| GitInventoryError::MalformedOutput)?;
            if !branch.starts_with("refs/heads/") || branch.len() == "refs/heads/".len() {
                return Err(GitInventoryError::MalformedOutput);
            }
            self.branch = Some(branch["refs/heads/".len()..].to_owned());
        } else if field == b"detached" {
            if self.detached {
                return Err(GitInventoryError::MalformedOutput);
            }
            self.detached = true;
        } else if field == b"locked" || field.starts_with(b"locked ") {
            if self.locked {
                return Err(GitInventoryError::MalformedOutput);
            }
            self.locked = true;
        } else if field == b"prunable" || field.starts_with(b"prunable ") || field == b"bare" {
            return Err(GitInventoryError::UnsupportedWorktree);
        } else {
            return Err(GitInventoryError::MalformedOutput);
        }
        Ok(())
    }

    fn finish(self) -> Result<WorktreeRecord, GitInventoryError> {
        if !self.head_seen {
            return Err(GitInventoryError::MalformedOutput);
        }
        let branch = match (self.branch, self.detached, self.head.is_some()) {
            (Some(branch), false, true) => GitBranchState::Attached(branch),
            (Some(branch), false, false) => GitBranchState::Unborn(branch),
            (None, true, true) => GitBranchState::Detached,
            _ => return Err(GitInventoryError::MalformedOutput),
        };
        let head = self.head.map_or(GitHeadState::Unborn, GitHeadState::Commit);
        Ok(WorktreeRecord {
            path: self.path,
            head,
            branch,
            locked: self.locked,
        })
    }
}

impl WorktreeRecord {
    pub(super) fn into_inventory(self, canonical_path: PathBuf) -> GitWorktreeInventory {
        GitWorktreeInventory {
            path: canonical_path,
            head: self.head,
            branch: self.branch,
            locked: self.locked,
        }
    }
}
