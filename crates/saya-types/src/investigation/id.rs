//! The investigation identifier: how a saved investigation is named on disk.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::InvestigationError;

/// How long an id may be: it is a filename stem, so it stays within every
/// filesystem's component limits.
pub(super) const MAX_ID_CHARS: usize = 64;
const MAX_ID_SLUG_CHARS: usize = 48;
const ID_HASH_BYTES: usize = 4;

/// Opaque, validated investigation id: lowercase ASCII that is a safe
/// filename stem (`<id>.json`) on every platform.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct InvestigationId(String);

impl InvestigationId {
    pub fn parse(value: &str) -> Result<Self, InvestigationError> {
        if (1..=MAX_ID_CHARS).contains(&value.len())
            && value
                .bytes()
                .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-'))
            && !value.starts_with('-')
            && !value.ends_with('-')
            && !value.contains("--")
        {
            Ok(Self(value.to_owned()))
        } else {
            Err(InvestigationError::InvalidId)
        }
    }

    /// Derives a stable id: the name becomes a readable slug, the rest a
    /// hash, so same-name saves never collide with an older revision. The
    /// slug keeps every rule `parse` enforces, so this constructs directly.
    pub fn derive(name: &str, sql: &str, created_unix_ms: i64) -> Self {
        let mut id = String::new();
        for ch in name.chars() {
            if ch.is_ascii_alphanumeric() {
                id.extend(ch.to_lowercase());
            } else if !id.ends_with('-') {
                id.push('-');
            }
        }
        let trimmed = id.trim_matches('-');
        let mut id: String = trimmed.chars().take(MAX_ID_SLUG_CHARS).collect();
        if id.is_empty() {
            id.push_str("investigation");
        }
        id.truncate(id.trim_end_matches('-').len());
        let mut hash = Sha256::new();
        hash.update(name.as_bytes());
        hash.update([0u8]);
        hash.update(sql.as_bytes());
        hash.update([0u8]);
        hash.update(created_unix_ms.to_string().as_bytes());
        id.push('-');
        for byte in &hash.finalize()[..ID_HASH_BYTES] {
            id.push_str(&format!("{byte:02x}"));
        }
        Self(id)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for InvestigationId {
    type Error = InvestigationError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<InvestigationId> for String {
    fn from(value: InvestigationId) -> Self {
        value.0
    }
}
