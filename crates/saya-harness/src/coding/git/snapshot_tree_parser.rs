use std::path::PathBuf;

use saya_types::GitObjectId;

use super::{
    command::GitObjectFormat,
    parser::{InventoryBudget, repository_path},
    repository::GitInventoryError,
    status_fields::parse_oid,
};

pub(super) struct GitTreeEntry {
    pub(super) path: PathBuf,
    pub(super) mode: u32,
    pub(super) object_id: GitObjectId,
    pub(super) declared_bytes: u64,
}

pub(super) fn parse_tree_entries(
    bytes: &[u8],
    format: GitObjectFormat,
    budget: &mut InventoryBudget,
) -> Result<Vec<GitTreeEntry>, GitInventoryError> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    if !bytes.ends_with(b"\0") {
        return Err(GitInventoryError::MalformedOutput);
    }
    let mut entries = Vec::new();
    for record in bytes[..bytes.len() - 1].split(|byte| *byte == 0) {
        if record.is_empty() {
            return Err(GitInventoryError::MalformedOutput);
        }
        if entries.len() >= super::parser::MAX_SNAPSHOT_INPUT_PATHS {
            return Err(GitInventoryError::TooLarge);
        }
        let tab = record
            .iter()
            .position(|byte| *byte == b'\t')
            .ok_or(GitInventoryError::MalformedOutput)?;
        let mut fields = record[..tab]
            .split(|byte| byte.is_ascii_whitespace())
            .filter(|field| !field.is_empty());
        let mode_bytes = fields.next().ok_or(GitInventoryError::MalformedOutput)?;
        let kind = fields.next().ok_or(GitInventoryError::MalformedOutput)?;
        let oid_bytes = fields.next().ok_or(GitInventoryError::MalformedOutput)?;
        if kind != b"blob" {
            return Err(GitInventoryError::UnsupportedRepositoryState);
        }
        let mode = u32::from_str_radix(
            std::str::from_utf8(mode_bytes).map_err(|_| GitInventoryError::MalformedOutput)?,
            8,
        )
        .map_err(|_| GitInventoryError::MalformedOutput)?;
        let object_id = parse_oid(oid_bytes, format)?;
        let size_bytes = fields.next().ok_or(GitInventoryError::MalformedOutput)?;
        if fields.next().is_some() {
            return Err(GitInventoryError::MalformedOutput);
        }
        let declared_bytes = std::str::from_utf8(size_bytes)
            .map_err(|_| GitInventoryError::MalformedOutput)?
            .parse::<u64>()
            .map_err(|_| GitInventoryError::MalformedOutput)?;
        let raw_path = &record[tab + 1..];
        let path = repository_path(raw_path)?;
        reject_git_components(&path)?;
        budget.add_path(raw_path)?;
        entries.push(GitTreeEntry {
            path,
            mode,
            object_id,
            declared_bytes,
        });
    }
    Ok(entries)
}

pub(super) fn reject_git_components(path: &std::path::Path) -> Result<(), GitInventoryError> {
    if path.components().any(|component| {
        component
            .as_os_str()
            .to_str()
            .is_some_and(|name| name.eq_ignore_ascii_case(".git"))
    }) {
        Err(GitInventoryError::UnsafePath)
    } else {
        Ok(())
    }
}
