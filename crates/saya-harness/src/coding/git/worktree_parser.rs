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

pub(super) fn parse_worktrees_z(
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
                return Err(GitInventoryError::BoundExceeded {
                    bound: "worktree records",
                    measured: records.len() as u64,
                    limit: MAX_INVENTORY_PATHS as u64,
                });
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

/// Parse the newline-terminated `--porcelain` output Git versions before 2.36
/// produce. A path containing a newline cannot be represented unambiguously in
/// that format, so such a listing is refused rather than partially parsed.
pub(super) fn parse_worktrees_lines(
    bytes: &[u8],
    format: GitObjectFormat,
) -> Result<Vec<WorktreeRecord>, GitInventoryError> {
    if !bytes.ends_with(b"\n\n") {
        return Err(GitInventoryError::MalformedOutput);
    }
    let mut records = Vec::new();
    let mut current: Option<PartialWorktree> = None;
    for line in bytes[..bytes.len() - 1].split(|byte| *byte == b'\n') {
        if line.is_empty() {
            let record = current.take().ok_or(GitInventoryError::MalformedOutput)?;
            records.push(record.finish()?);
            if records.len() > MAX_INVENTORY_PATHS {
                return Err(GitInventoryError::BoundExceeded {
                    bound: "worktree records",
                    measured: records.len() as u64,
                    limit: MAX_INVENTORY_PATHS as u64,
                });
            }
            continue;
        }
        if let Some(path) = line.strip_prefix(b"worktree ") {
            if current.is_some() || path.is_empty() {
                return Err(GitInventoryError::MalformedOutput);
            }
            // Git C-quotes a path with unsafe bytes in this format. Saya does
            // not decode the quoting, so such a path is refused by name.
            if path.starts_with(b"\"") {
                return Err(GitInventoryError::UnsupportedWorktreePath);
            }
            let path = path_from_git_bytes(path)?;
            if !path.is_absolute() {
                return Err(GitInventoryError::UnsafePath);
            }
            current = Some(PartialWorktree::new(path));
            continue;
        }
        let record = current.as_mut().ok_or(GitInventoryError::MalformedOutput)?;
        // A newline inside a worktree path splits it across lines. The
        // continuation is not a field label, so refusing it here keeps the
        // listing from being parsed only in part.
        if !known_field(line) {
            return Err(GitInventoryError::UnsupportedWorktreePath);
        }
        record.push_field(line, format)?;
    }
    if current.is_some() {
        return Err(GitInventoryError::MalformedOutput);
    }
    if records.is_empty() {
        return Err(GitInventoryError::MalformedOutput);
    }
    Ok(records)
}

fn known_field(field: &[u8]) -> bool {
    field.starts_with(b"HEAD ")
        || field.starts_with(b"branch ")
        || field == b"detached"
        || field == b"locked"
        || field.starts_with(b"locked ")
        || field == b"prunable"
        || field.starts_with(b"prunable ")
        || field == b"bare"
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

#[cfg(test)]
mod tests {
    use super::*;

    const FIRST: &str = "1111111111111111111111111111111111111111";
    const SECOND: &str = "2222222222222222222222222222222222222222";

    fn z_listing() -> Vec<u8> {
        format!(
            "worktree /first\0HEAD {FIRST}\0branch refs/heads/main\0\0\
             worktree /second\0HEAD {SECOND}\0detached\0\0"
        )
        .into_bytes()
    }

    fn line_listing() -> Vec<u8> {
        format!(
            "worktree /first\nHEAD {FIRST}\nbranch refs/heads/main\n\n\
             worktree /second\nHEAD {SECOND}\ndetached\n\n"
        )
        .into_bytes()
    }

    #[test]
    fn both_terminations_parse_the_same_worktree_set() {
        let zero = parse_worktrees_z(&z_listing(), GitObjectFormat::Sha1).unwrap();
        let lines = parse_worktrees_lines(&line_listing(), GitObjectFormat::Sha1).unwrap();
        assert_eq!(zero.len(), 2);
        assert_eq!(lines.len(), 2);
        for (zero, line) in zero.iter().zip(&lines) {
            assert_eq!(zero.path, line.path);
            assert_eq!(zero.head, line.head);
            assert_eq!(zero.branch, line.branch);
            assert_eq!(zero.locked, line.locked);
        }
        assert_eq!(zero[0].branch, GitBranchState::Attached("main".to_owned()));
        assert_eq!(zero[1].branch, GitBranchState::Detached);
    }

    #[test]
    fn a_newline_in_a_worktree_path_is_refused_by_name() {
        let bytes = format!(
            "worktree /first\nHEAD {FIRST}\nbranch refs/heads/main\n\n\
             worktree /bad\npath\nHEAD {SECOND}\ndetached\n\n"
        );
        assert!(matches!(
            parse_worktrees_lines(bytes.as_bytes(), GitObjectFormat::Sha1),
            Err(GitInventoryError::UnsupportedWorktreePath)
        ));
    }

    #[test]
    fn a_c_quoted_worktree_path_is_refused_by_name() {
        let bytes = format!("worktree \"/bad\\npath\"\nHEAD {FIRST}\n\n");
        assert!(matches!(
            parse_worktrees_lines(bytes.as_bytes(), GitObjectFormat::Sha1),
            Err(GitInventoryError::UnsupportedWorktreePath)
        ));
    }

    #[test]
    fn an_unterminated_line_listing_is_malformed() {
        let bytes = format!("worktree /first\nHEAD {FIRST}\nbranch refs/heads/main\n");
        assert!(matches!(
            parse_worktrees_lines(bytes.as_bytes(), GitObjectFormat::Sha1),
            Err(GitInventoryError::MalformedOutput)
        ));
    }
}
