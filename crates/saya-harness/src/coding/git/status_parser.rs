use std::collections::HashSet;

use super::{
    command::{GitConfigRisk, GitObjectFormat},
    inventory_types::GitPathStatus,
    parser::{InventoryBudget, repository_path},
    repository::{GitHeadState, GitInventoryError},
    status_fields::{fixed_fields, parse_oid, parse_xy, valid_signed_count, validate_modes},
};

pub(super) struct ParsedStatus {
    pub(super) head: GitHeadState,
    pub(super) branch: String,
    pub(super) paths: Vec<GitPathStatus>,
}

pub(super) fn parse_status(
    bytes: &[u8],
    format: GitObjectFormat,
    budget: &mut InventoryBudget,
) -> Result<ParsedStatus, GitInventoryError> {
    if !bytes.ends_with(&[0]) || bytes.len() < 2 {
        return Err(GitInventoryError::MalformedOutput);
    }
    let mut head = None;
    let mut branch = None;
    let mut upstream_seen = false;
    let mut ahead_behind_seen = false;
    let mut paths = Vec::new();
    let mut primary_paths = HashSet::new();
    let mut records = bytes[..bytes.len() - 1].split(|byte| *byte == 0);
    while let Some(record) = records.next() {
        if record.is_empty() {
            return Err(GitInventoryError::MalformedOutput);
        }
        if record.starts_with(b"# ") {
            parse_header(
                record,
                format,
                &mut head,
                &mut branch,
                &mut upstream_seen,
                &mut ahead_behind_seen,
            )?;
            continue;
        }
        let status = match record.first() {
            Some(b'1') => parse_changed(record, 8, format, false, budget)?,
            Some(b'2') => {
                let mut status = parse_changed(record, 9, format, true, budget)?;
                let original = records
                    .next()
                    .filter(|path| !path.is_empty())
                    .ok_or(GitInventoryError::MalformedOutput)?;
                let original_path = repository_path(original)?;
                budget.add_path(original)?;
                status.original_path = Some(original_path);
                status
            }
            Some(b'u') => parse_unmerged(record, format, budget)?,
            Some(b'?') => parse_simple(record, b"? ", true, budget)?,
            Some(b'!') => parse_simple(record, b"! ", false, budget)?,
            _ => return Err(GitInventoryError::MalformedOutput),
        };
        if !primary_paths.insert(status.path.as_os_str().as_encoded_bytes().to_vec())
            || status.original_path.as_ref().is_some_and(|path| {
                !primary_paths.insert(path.as_os_str().as_encoded_bytes().to_vec())
            })
        {
            return Err(GitInventoryError::MalformedOutput);
        }
        paths.push(status);
    }
    Ok(ParsedStatus {
        head: head.ok_or(GitInventoryError::MalformedOutput)?,
        branch: branch.ok_or(GitInventoryError::MalformedOutput)?,
        paths,
    })
}

fn parse_header(
    record: &[u8],
    format: GitObjectFormat,
    head: &mut Option<GitHeadState>,
    branch: &mut Option<String>,
    upstream_seen: &mut bool,
    ahead_behind_seen: &mut bool,
) -> Result<(), GitInventoryError> {
    if let Some(value) = record.strip_prefix(b"# branch.oid ") {
        if head.is_some() {
            return Err(GitInventoryError::MalformedOutput);
        }
        *head = Some(if value == b"(initial)" {
            GitHeadState::Unborn
        } else {
            GitHeadState::Commit(parse_oid(value, format)?)
        });
    } else if let Some(value) = record.strip_prefix(b"# branch.head ") {
        if branch.is_some() || value.is_empty() {
            return Err(GitInventoryError::MalformedOutput);
        }
        *branch = Some(
            std::str::from_utf8(value)
                .map_err(|_| GitInventoryError::MalformedOutput)?
                .to_owned(),
        );
    } else if let Some(value) = record.strip_prefix(b"# branch.upstream ") {
        if *upstream_seen || value.is_empty() {
            return Err(GitInventoryError::MalformedOutput);
        }
        *upstream_seen = true;
    } else if let Some(value) = record.strip_prefix(b"# branch.ab ") {
        let fields: Vec<_> = value.split(|byte| *byte == b' ').collect();
        if *ahead_behind_seen
            || fields.len() != 2
            || !valid_signed_count(fields[0])
            || !valid_signed_count(fields[1])
        {
            return Err(GitInventoryError::MalformedOutput);
        }
        *ahead_behind_seen = true;
    } else {
        return Err(GitInventoryError::MalformedOutput);
    }
    Ok(())
}

fn parse_changed(
    record: &[u8],
    field_count: usize,
    format: GitObjectFormat,
    renamed: bool,
    budget: &mut InventoryBudget,
) -> Result<GitPathStatus, GitInventoryError> {
    let (fields, path) = fixed_fields(record, field_count)?;
    let xy = fields.get(1).ok_or(GitInventoryError::MalformedOutput)?;
    let submodule = fields.get(2).ok_or(GitInventoryError::MalformedOutput)?;
    if xy.len() != 2 || submodule.len() != 4 {
        return Err(GitInventoryError::MalformedOutput);
    }
    if submodule[0] == b'S' {
        return Err(GitInventoryError::Git(
            super::command::GitError::UnsupportedConfiguration(GitConfigRisk::Submodule),
        ));
    } else if submodule != b"N..." {
        return Err(GitInventoryError::MalformedOutput);
    }
    validate_modes(&fields[3..6])?;
    for oid in &fields[6..8] {
        let _ = parse_oid(oid, format)?;
    }
    if renamed {
        let score = fields.get(8).ok_or(GitInventoryError::MalformedOutput)?;
        if score.len() < 2
            || !matches!(score[0], b'R' | b'C')
            || !score[1..].iter().all(u8::is_ascii_digit)
            || std::str::from_utf8(&score[1..])
                .ok()
                .and_then(|value| value.parse::<u8>().ok())
                .is_none_or(|value| value > 100)
        {
            return Err(GitInventoryError::MalformedOutput);
        }
    }
    let path = repository_path(path)?;
    budget.add_path(path.as_os_str().as_encoded_bytes())?;
    let (staged, unstaged, conflicted) = parse_xy(xy)?;
    Ok(GitPathStatus {
        path,
        original_path: None,
        staged,
        unstaged,
        staged_added: xy[0] == b'A',
        unstaged_added: xy[1] == b'A',
        staged_deleted: xy[0] == b'D',
        unstaged_deleted: xy[1] == b'D',
        untracked: false,
        untracked_directory: false,
        ignored: false,
        conflicted,
    })
}

fn parse_unmerged(
    record: &[u8],
    format: GitObjectFormat,
    budget: &mut InventoryBudget,
) -> Result<GitPathStatus, GitInventoryError> {
    let (fields, path) = fixed_fields(record, 10)?;
    let xy = fields.get(1).ok_or(GitInventoryError::MalformedOutput)?;
    let submodule = fields.get(2).ok_or(GitInventoryError::MalformedOutput)?;
    if xy.len() != 2 || submodule.len() != 4 {
        return Err(GitInventoryError::MalformedOutput);
    }
    if submodule[0] == b'S' {
        return Err(GitInventoryError::Git(
            super::command::GitError::UnsupportedConfiguration(GitConfigRisk::Submodule),
        ));
    } else if submodule != b"N..." {
        return Err(GitInventoryError::MalformedOutput);
    }
    validate_modes(&fields[3..7])?;
    for oid in &fields[7..10] {
        let _ = parse_oid(oid, format)?;
    }
    let path = repository_path(path)?;
    budget.add_path(path.as_os_str().as_encoded_bytes())?;
    let (staged, unstaged, _) = parse_xy(xy)?;
    Ok(GitPathStatus {
        path,
        original_path: None,
        staged,
        unstaged,
        staged_added: false,
        unstaged_added: false,
        staged_deleted: false,
        unstaged_deleted: false,
        untracked: false,
        untracked_directory: false,
        ignored: false,
        conflicted: true,
    })
}

fn parse_simple(
    record: &[u8],
    prefix: &[u8],
    untracked: bool,
    budget: &mut InventoryBudget,
) -> Result<GitPathStatus, GitInventoryError> {
    let path_bytes = record
        .strip_prefix(prefix)
        .filter(|path| !path.is_empty())
        .ok_or(GitInventoryError::MalformedOutput)?;
    let path = repository_path(path_bytes)?;
    budget.add_path(path_bytes)?;
    Ok(GitPathStatus {
        path,
        original_path: None,
        staged: false,
        unstaged: false,
        staged_added: false,
        unstaged_added: false,
        staged_deleted: false,
        unstaged_deleted: false,
        untracked,
        untracked_directory: untracked && path_bytes.ends_with(b"/"),
        ignored: !untracked,
        conflicted: false,
    })
}
