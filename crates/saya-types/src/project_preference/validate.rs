//! Bound enforcement for explicit project preference records: the checks
//! every record must pass however it arrived — the version, the id and
//! worktree shapes, the text, the timestamps, and the record's own
//! serialized size — plus the edit and set-level admission rules. Records
//! built in code and records read back as JSON validate through the same
//! gate.

use std::collections::HashSet;

use serde::Deserialize;

use super::{
    MAX_PREFERENCE_RECALL_BYTES, MAX_PREFERENCE_RECORD_BYTES, MAX_PREFERENCE_TEXT_BYTES,
    MAX_PROJECT_PREFERENCES, PROJECT_PREFERENCE_VERSION, PreferenceError, ProjectPreference,
};

impl ProjectPreference {
    /// Reads a preference from serialized bytes: the byte cap before any
    /// parsing, then a lenient probe of only `version` so an unknown version
    /// fails precisely, then the strict parse with every bound re-checked.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, PreferenceError> {
        if bytes.len() > MAX_PREFERENCE_RECORD_BYTES {
            return Err(PreferenceError::Oversize(bytes.len()));
        }
        #[derive(Deserialize)]
        struct Probe {
            version: Option<u32>,
        }
        let probe: Probe = serde_json::from_slice(bytes).map_err(|_| PreferenceError::Malformed)?;
        if let Some(version) = probe.version.filter(|v| *v != PROJECT_PREFERENCE_VERSION) {
            return Err(PreferenceError::UnsupportedVersion(version));
        }
        let record: Self = serde_json::from_slice(bytes).map_err(|_| PreferenceError::Malformed)?;
        record.validate()?;
        Ok(record)
    }

    /// Serializes after re-validating, so a record mutated in code cannot
    /// leave this crate in a shape it would refuse on the way back in.
    pub fn to_json_bytes(&self) -> Result<Vec<u8>, PreferenceError> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|_| PreferenceError::Malformed)
    }

    /// Re-checks every rule for a record that may have arrived as JSON,
    /// which skips no gate: the version, the id and worktree shapes, the
    /// text, the timestamps, and the record's own serialized size.
    pub fn validate(&self) -> Result<(), PreferenceError> {
        if self.version != PROJECT_PREFERENCE_VERSION {
            return Err(PreferenceError::UnsupportedVersion(self.version));
        }
        // Re-checked: the derived Deserialize on a newtype bypasses parse.
        if !is_preference_id(self.id.as_str()) {
            return Err(PreferenceError::InvalidId);
        }
        if !is_worktree_id(self.worktree.as_str()) {
            return Err(PreferenceError::InvalidWorktree);
        }
        validate_text(&self.text)?;
        if self.created_unix_ms < 0 {
            return Err(PreferenceError::NegativeTimestamp);
        }
        if self.updated_unix_ms < self.created_unix_ms {
            return Err(PreferenceError::TimestampOrder);
        }
        let bytes = serde_json::to_vec(self).map_err(|_| PreferenceError::Malformed)?;
        if bytes.len() > MAX_PREFERENCE_RECORD_BYTES {
            return Err(PreferenceError::Oversize(bytes.len()));
        }
        Ok(())
    }
}

/// Set-level admission for one worktree's preferences: every item shares one
/// worktree, ids are unique, the count fits [`MAX_PROJECT_PREFERENCES`], and
/// the summed text fits [`MAX_PREFERENCE_RECALL_BYTES`].
pub fn validate_set(items: &[ProjectPreference]) -> Result<(), PreferenceError> {
    if items.len() > MAX_PROJECT_PREFERENCES {
        return Err(PreferenceError::SetTooManyItems(items.len()));
    }
    let mut worktree: Option<&str> = None;
    let mut ids = HashSet::with_capacity(items.len());
    let mut total = 0usize;
    for item in items {
        if let Some(first) = worktree {
            if first != item.worktree.as_str() {
                return Err(PreferenceError::MixedWorktrees);
            }
        } else {
            worktree = Some(item.worktree.as_str());
        }
        if !ids.insert(item.id.as_str()) {
            return Err(PreferenceError::DuplicateId);
        }
        total += item.text.len();
    }
    if total > MAX_PREFERENCE_RECALL_BYTES {
        return Err(PreferenceError::RecallOversize(total));
    }
    Ok(())
}

/// A preference's text is usable guidance: non-empty after trimming, at most
/// [`MAX_PREFERENCE_TEXT_BYTES`] bytes, and free of control characters other
/// than newline and tab — no NUL, no carriage return, no escape sequences
/// that could smuggle a command shape past a renderer.
pub(super) fn validate_text(text: &str) -> Result<(), PreferenceError> {
    if text.trim().is_empty() {
        return Err(PreferenceError::TextEmpty);
    }
    if text.len() > MAX_PREFERENCE_TEXT_BYTES {
        return Err(PreferenceError::TextOversize(text.len()));
    }
    if text
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(PreferenceError::TextControlCharacter);
    }
    Ok(())
}

/// A worktree id is exactly `wt-` followed by 64 lowercase hex characters.
pub(super) fn is_worktree_id(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("wt-") else {
        return false;
    };
    rest.len() == 64
        && rest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

/// A preference id is exactly `pp-` followed by 32 lowercase hex characters.
pub(super) fn is_preference_id(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("pp-") else {
        return false;
    };
    rest.len() == 32
        && rest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}
