//! Bound enforcement for session continuation records: the checks every
//! record and fact must pass, however it arrived. Records built in code and
//! records read back as JSON validate through the same gate. The prose bounds
//! live with the prose in `boundary`; the fact types live in `facts`.

use std::collections::HashSet;

use super::boundary::{validate_narrative, validate_user_notes};
use super::facts::{CheckFact, FactProvenance, FileFact};
use super::{
    CONTINUATION_VERSION, ContinuationError, MAX_CHECK_FACTS, MAX_CHECK_LABEL_CHARS,
    MAX_CONTINUATION_BYTES, MAX_FACT_PATH_BYTES, MAX_FILE_FACTS, SessionContinuationV1,
};
use crate::is_name_shaped;

impl SessionContinuationV1 {
    /// Re-checks every bound for a record that may have arrived as JSON,
    /// which skips no gate: the version, the source digest, the narrative and
    /// user notes, every fact, and the record's own serialized size.
    pub fn validate(&self) -> Result<(), ContinuationError> {
        if self.version != CONTINUATION_VERSION {
            return Err(ContinuationError::UnsupportedVersion(self.version));
        }
        if !is_sha256_hex(&self.source.source_sha256) {
            return Err(ContinuationError::InvalidDigest);
        }
        if let Some(narrative) = &self.narrative {
            validate_narrative(narrative)?;
        }
        if let Some(notes) = &self.user_notes {
            validate_user_notes(notes)?;
        }
        validate_files(&self.files)?;
        validate_checks(&self.checks)?;
        let bytes = serde_json::to_vec(self).map_err(|_| ContinuationError::Malformed)?;
        if bytes.len() > MAX_CONTINUATION_BYTES {
            return Err(ContinuationError::Oversize(bytes.len()));
        }
        Ok(())
    }
}

fn validate_files(files: &[FileFact]) -> Result<(), ContinuationError> {
    if files.len() > MAX_FILE_FACTS {
        return Err(ContinuationError::TooManyFiles(files.len()));
    }
    let mut seen = HashSet::with_capacity(files.len());
    for fact in files {
        validate_fact_path(&fact.path)?;
        if let Some(digest) = &fact.sha256
            && !is_sha256_hex(digest)
        {
            return Err(ContinuationError::InvalidDigest);
        }
        if fact.observed_unix_ms < 0 {
            return Err(ContinuationError::NegativeTimestamp);
        }
        validate_provenance(&fact.provenance)?;
        if !seen.insert(fact.path.as_str()) {
            return Err(ContinuationError::DuplicateFilePath);
        }
    }
    Ok(())
}

/// A check label is bounded like a task title: usable text, no control
/// characters.
fn validate_checks(checks: &[CheckFact]) -> Result<(), ContinuationError> {
    if checks.len() > MAX_CHECK_FACTS {
        return Err(ContinuationError::TooManyChecks(checks.len()));
    }
    for fact in checks {
        if fact.label.trim().is_empty()
            || fact.label.chars().count() > MAX_CHECK_LABEL_CHARS
            || fact.label.chars().any(char::is_control)
        {
            return Err(ContinuationError::InvalidCheckLabel);
        }
        if fact.observed_unix_ms < 0 {
            return Err(ContinuationError::NegativeTimestamp);
        }
        validate_provenance(&fact.provenance)?;
    }
    Ok(())
}

/// A fact path names one file relative to the workspace root: never absolute,
/// never a `..` escape, and never a non-canonical shape (`src/`, `a//b`) that
/// would make duplicate detection and downstream joining ambiguous.
fn validate_fact_path(path: &str) -> Result<(), ContinuationError> {
    if path.is_empty() {
        return Err(ContinuationError::InvalidFactPath);
    }
    if path.len() > MAX_FACT_PATH_BYTES {
        return Err(ContinuationError::FactPathOversize(path.len()));
    }
    if path.chars().any(char::is_control) || is_absolute_path(path) {
        return Err(ContinuationError::InvalidFactPath);
    }
    if path
        .split(['/', '\\'])
        .any(|component| component.is_empty() || component == "..")
    {
        return Err(ContinuationError::InvalidFactPath);
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

fn validate_provenance(provenance: &FactProvenance) -> Result<(), ContinuationError> {
    match provenance {
        FactProvenance::ToolObserved { tool } if is_name_shaped(tool) => Ok(()),
        FactProvenance::ToolObserved { .. } => Err(ContinuationError::InvalidToolName),
    }
}

/// A digest is 64 lowercase hex characters — the shape `SchemaFingerprint`
/// enforces. A digest detects drift; it does not authenticate.
fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}
