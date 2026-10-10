use std::{collections::HashSet, path::PathBuf};

use saya_types::GitObjectId;

use super::{
    command::{GitConfigRisk, GitError, GitObjectFormat},
    parser::{InventoryBudget, path_from_git_bytes, repository_path},
    repository::GitInventoryError,
};

pub(super) fn validate_index_entries(
    bytes: &[u8],
    format: GitObjectFormat,
    budget: &mut InventoryBudget,
) -> Result<Vec<PathBuf>, GitInventoryError> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    if !bytes.ends_with(&[0]) {
        return Err(GitInventoryError::MalformedOutput);
    }
    let mut stages = HashSet::new();
    let mut paths = Vec::new();
    for record in bytes[..bytes.len() - 1].split(|byte| *byte == 0) {
        if record.is_empty() {
            return Err(GitInventoryError::MalformedOutput);
        }
        let tab = record
            .iter()
            .position(|byte| *byte == b'\t')
            .ok_or(GitInventoryError::MalformedOutput)?;
        let mut fields = record[..tab]
            .split(|byte| byte.is_ascii_whitespace())
            .filter(|field| !field.is_empty());
        let mode = fields.next().ok_or(GitInventoryError::MalformedOutput)?;
        let object = fields.next().ok_or(GitInventoryError::MalformedOutput)?;
        let stage = fields.next().ok_or(GitInventoryError::MalformedOutput)?;
        if fields.next().is_some() {
            return Err(GitInventoryError::MalformedOutput);
        }
        if mode == b"160000" {
            return Err(GitInventoryError::Git(GitError::UnsupportedConfiguration(
                GitConfigRisk::Submodule,
            )));
        }
        if mode != b"100644" && mode != b"100755" && mode != b"120000" {
            return Err(GitInventoryError::Git(GitError::UnsupportedTreeEntry));
        }
        let object = GitObjectId::parse(
            std::str::from_utf8(object).map_err(|_| GitInventoryError::MalformedOutput)?,
        )
        .map_err(|_| GitInventoryError::MalformedOutput)?;
        let expected_len = match format {
            GitObjectFormat::Sha1 => 40,
            GitObjectFormat::Sha256 => 64,
        };
        if object.as_str().len() != expected_len {
            return Err(GitInventoryError::MalformedOutput);
        }
        let stage = match stage {
            b"0" => 0,
            b"1" => 1,
            b"2" => 2,
            b"3" => 3,
            _ => return Err(GitInventoryError::MalformedOutput),
        };
        let path = &record[tab + 1..];
        let _ = repository_path(path)?;
        budget.add_path(path)?;
        paths.push(path_from_git_bytes(path)?);
        if !stages.insert((path.to_vec(), stage)) {
            return Err(GitInventoryError::MalformedOutput);
        }
    }
    Ok(paths)
}

pub(super) fn reject_sparse_index(bytes: &[u8]) -> Result<(), GitInventoryError> {
    if bytes.is_empty() {
        return Ok(());
    }
    if !bytes.ends_with(&[0]) {
        return Err(GitInventoryError::MalformedOutput);
    }
    if bytes[..bytes.len() - 1]
        .split(|byte| *byte == 0)
        .any(|record| {
            record
                .first()
                .is_some_and(|tag| matches!(*tag, b'S' | b's'))
                && record.get(1) == Some(&b' ')
        })
    {
        return Err(GitInventoryError::Git(GitError::UnsupportedConfiguration(
            GitConfigRisk::SparseCheckout,
        )));
    }
    Ok(())
}
