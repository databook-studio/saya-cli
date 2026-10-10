use saya_types::GitObjectId;

use super::{
    command::{GitConfigRisk, GitError, GitObjectFormat},
    repository::GitInventoryError,
};

pub(super) fn parse_xy(xy: &[u8]) -> Result<(bool, bool, bool), GitInventoryError> {
    let valid = |byte: u8| matches!(byte, b'.' | b'M' | b'T' | b'A' | b'D' | b'R' | b'C' | b'U');
    if xy.len() != 2 || !valid(xy[0]) || !valid(xy[1]) || xy == b".." {
        return Err(GitInventoryError::MalformedOutput);
    }
    let conflict = xy[0] == b'U' || xy[1] == b'U';
    Ok((xy[0] != b'.', xy[1] != b'.', conflict))
}

pub(super) fn fixed_fields(
    record: &[u8],
    count: usize,
) -> Result<(Vec<&[u8]>, &[u8]), GitInventoryError> {
    let mut rest = record;
    let mut fields = Vec::with_capacity(count);
    for index in 0..count {
        let separator = rest
            .iter()
            .position(|byte| *byte == b' ')
            .ok_or(GitInventoryError::MalformedOutput)?;
        if separator == 0 {
            return Err(GitInventoryError::MalformedOutput);
        }
        fields.push(&rest[..separator]);
        rest = &rest[separator + 1..];
        if index + 1 == count {
            return Ok((fields, rest));
        }
    }
    Err(GitInventoryError::MalformedOutput)
}

pub(super) fn validate_modes(modes: &[&[u8]]) -> Result<(), GitInventoryError> {
    for mode in modes {
        if *mode != b"000000"
            && *mode != b"100644"
            && *mode != b"100755"
            && *mode != b"120000"
            && *mode != b"160000"
        {
            return Err(GitInventoryError::MalformedOutput);
        }
        if *mode == b"160000" {
            return Err(GitInventoryError::Git(GitError::UnsupportedConfiguration(
                GitConfigRisk::Submodule,
            )));
        }
    }
    Ok(())
}

pub(super) fn valid_signed_count(value: &[u8]) -> bool {
    value
        .first()
        .is_some_and(|byte| matches!(*byte, b'+' | b'-'))
        && value.len() > 1
        && value[1..].iter().all(u8::is_ascii_digit)
}

pub(super) fn parse_oid(
    bytes: &[u8],
    format: GitObjectFormat,
) -> Result<GitObjectId, GitInventoryError> {
    let value = std::str::from_utf8(bytes).map_err(|_| GitInventoryError::MalformedOutput)?;
    let object = GitObjectId::parse(value).map_err(|_| GitInventoryError::MalformedOutput)?;
    let expected_len = match format {
        GitObjectFormat::Sha1 => 40,
        GitObjectFormat::Sha256 => 64,
    };
    if object.as_str().len() != expected_len {
        return Err(GitInventoryError::MalformedOutput);
    }
    Ok(object)
}
