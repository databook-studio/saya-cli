//! How a checkpoint record arrives and gets gated: the byte cap, the lenient
//! version probe, the strict parse, and the consistency checks every record
//! must pass however it arrived — the state/after-image identity, digest
//! shape, file size, path shape, timestamps, and the record's own serialized
//! size. Records built in code validate through the same gate. The policy and
//! quota checks live in `policy`.

use serde::Deserialize;

use super::{
    BeforeImage, CHECKPOINT_VERSION, CheckpointError, CheckpointState, EditCheckpoint, FileVersion,
    MAX_CHECKPOINT_FILE_BYTES, MAX_CHECKPOINT_PATH_BYTES, MAX_CHECKPOINT_RECORD_BYTES,
};

impl EditCheckpoint {
    /// Reads a checkpoint from serialized bytes: the byte cap before any
    /// parsing, then a lenient probe of only `version` so an unknown version
    /// fails precisely, then the strict parse with every bound re-checked.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, CheckpointError> {
        if bytes.len() > MAX_CHECKPOINT_RECORD_BYTES {
            return Err(CheckpointError::Oversize(bytes.len()));
        }
        #[derive(Deserialize)]
        struct Probe {
            version: Option<u32>,
        }
        let probe: Probe = serde_json::from_slice(bytes).map_err(|_| CheckpointError::Malformed)?;
        if let Some(version) = probe.version.filter(|v| *v != CHECKPOINT_VERSION) {
            return Err(CheckpointError::UnsupportedVersion(version));
        }
        let record: Self = serde_json::from_slice(bytes).map_err(|_| CheckpointError::Malformed)?;
        record.validate()?;
        Ok(record)
    }

    /// Serializes after re-validating, so a record mutated in code cannot
    /// leave this crate in a shape it would refuse on the way back in.
    pub fn to_json_bytes(&self) -> Result<Vec<u8>, CheckpointError> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|_| CheckpointError::Malformed)
    }

    /// Re-checks every consistency rule for a record that may have arrived
    /// as JSON, which skips no gate: the version, the state/after-image
    /// identity, the digests, the file sizes, the path, the timestamps, and
    /// the record's own serialized size.
    pub fn validate(&self) -> Result<(), CheckpointError> {
        if self.version != CHECKPOINT_VERSION {
            return Err(CheckpointError::UnsupportedVersion(self.version));
        }
        // Re-checked: the derived Deserialize on `CheckpointId` bypasses parse.
        if !is_checkpoint_id(self.id.as_str()) {
            return Err(CheckpointError::InvalidId);
        }
        match &self.before {
            BeforeImage::Present(before) => validate_version(before)?,
            BeforeImage::Absent => {}
        }
        if let Some(after) = &self.after {
            validate_version(after)?;
        }
        match self.state {
            CheckpointState::Committed if self.after.is_none() => {
                return Err(CheckpointError::CommittedRequiresAfter);
            }
            CheckpointState::Prepared | CheckpointState::Uncertain if self.after.is_some() => {
                return Err(CheckpointError::StateDisallowsAfter);
            }
            _ => {}
        }
        validate_path(&self.path)?;
        if self.created_unix_ms < 0 {
            return Err(CheckpointError::NegativeTimestamp);
        }
        if self.updated_unix_ms < self.created_unix_ms {
            return Err(CheckpointError::TimestampOrder);
        }
        let bytes = serde_json::to_vec(self).map_err(|_| CheckpointError::Malformed)?;
        if bytes.len() > MAX_CHECKPOINT_RECORD_BYTES {
            return Err(CheckpointError::Oversize(bytes.len()));
        }
        Ok(())
    }
}

/// One recorded file version: its digest detects drift and does not
/// authenticate, and its size fits the per-file ceiling.
fn validate_version(version: &FileVersion) -> Result<(), CheckpointError> {
    if !is_sha256_hex(&version.sha256) {
        return Err(CheckpointError::InvalidDigest);
    }
    if version.size > MAX_CHECKPOINT_FILE_BYTES {
        return Err(CheckpointError::VersionTooLarge(version.size));
    }
    Ok(())
}

/// A checkpoint path names one file relative to the workspace root: never
/// absolute, never a `..` escape, and never a non-canonical shape (`src/`,
/// `a//b`) that would make downstream joining ambiguous.
fn validate_path(path: &str) -> Result<(), CheckpointError> {
    if path.is_empty() {
        return Err(CheckpointError::InvalidPath);
    }
    if path.len() > MAX_CHECKPOINT_PATH_BYTES {
        return Err(CheckpointError::PathOversize(path.len()));
    }
    if path.chars().any(char::is_control) || is_absolute_path(path) {
        return Err(CheckpointError::InvalidPath);
    }
    if path
        .split(['/', '\\'])
        .any(|component| component.is_empty() || component == "..")
    {
        return Err(CheckpointError::InvalidPath);
    }
    Ok(())
}

/// Absolute on any platform: a leading separator, or a Windows drive root
/// (`C:\` or `C:/`), which is absolute however it is written.
fn is_absolute_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.first() == Some(&b'/')
        || bytes.first() == Some(&b'\\')
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && (bytes[2] == b'/' || bytes[2] == b'\\'))
}

/// A checkpoint id is exactly `ck-` followed by 32 lowercase hex characters.
pub(super) fn is_checkpoint_id(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("ck-") else {
        return false;
    };
    rest.len() == 32
        && rest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

/// A digest is 64 lowercase hex characters — the shape `SchemaFingerprint`
/// enforces. A digest detects drift; it does not authenticate.
fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}
