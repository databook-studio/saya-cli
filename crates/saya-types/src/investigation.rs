//! The saved-investigation contract: a portable, versioned definition of one
//! bounded, read-only SQL investigation — what the investigation *is* and
//! nothing about any run against it: no credentials, results, transcripts,
//! grants, or machine-specific profile identity.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::dialect::SqlDialect;

pub const INVESTIGATION_FORMAT: &str = "saya.investigation";
pub const INVESTIGATION_FORMAT_VERSION: u32 = 1;
/// Document, field, and collection bounds; names count Unicode scalars.
pub const MAX_DEFINITION_BYTES: usize = 128 * 1024;
pub const MAX_SQL_BYTES: usize = 64 * 1024;
pub const MAX_NAME_CHARS: usize = 80;
pub const MAX_DESCRIPTION_BYTES: usize = 2048;
pub const MAX_OBJECTS: usize = 256;
pub const MAX_OBJECT_BYTES: usize = 256;
pub const MAX_CONNECTION_CHARS: usize = 64;
pub const MAX_FINGERPRINT_BYTES: usize = 128;

const MAX_ID_CHARS: usize = 64;
const MAX_ID_SLUG_CHARS: usize = 48;
const ID_HASH_BYTES: usize = 4;

#[rustfmt::skip] // stays a one-line guard; rustfmt would expand it to seven
macro_rules! ensure {
    ($cond:expr, $err:expr) => { if !$cond { return Err($err); } };
}

/// Why a saved investigation was rejected — never echoing SQL or description content.
#[derive(Debug, Clone, PartialEq, Error)]
#[non_exhaustive]
#[rustfmt::skip] // one line per rejection: the enum reads as the bound table it is
pub enum InvestigationError {
    #[error("investigation document is {0} bytes, over the {MAX_DEFINITION_BYTES}-byte limit")] Oversize(usize),
    #[error("unsupported investigation version {0}")] UnsupportedVersion(u32),
    #[error("document is not a saya investigation")] NotAnInvestigation,
    #[error("investigation document is not valid JSON")] Malformed,
    #[error("investigation id is not a valid identifier")] InvalidId,
    #[error("investigation revision must be at least 1")] InvalidRevision,
    #[error("name must be 1-{MAX_NAME_CHARS} characters after trimming")] InvalidName,
    #[error("value contains control characters")] ControlCharacter,
    #[error("description is too long")] DescriptionTooLong,
    #[error("sql must be non-empty and at most {MAX_SQL_BYTES} bytes")] InvalidSql,
    #[error("connection must be 1-{MAX_CONNECTION_CHARS} characters of [A-Za-z0-9_.-]")] InvalidConnection,
    #[error("object names must be 1-{MAX_OBJECT_BYTES} bytes")] InvalidObject,
    #[error("too many objects ({0})")] TooManyObjects(usize),
    #[error("objects must not contain duplicates")] DuplicateObject,
    #[error("schema fingerprint must be 1-{MAX_FINGERPRINT_BYTES} printable bytes")] InvalidFingerprint,
    #[error("updated timestamp is before created timestamp")] UpdatedBeforeCreated,
}

/// Opaque, validated investigation id: a safe filename stem (`<id>.json`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct InvestigationId(String);

impl InvestigationId {
    pub fn parse(value: &str) -> Result<Self, InvestigationError> {
        let ascii_id = |b: u8| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-');
        let well_shaped = (1..=MAX_ID_CHARS).contains(&value.len())
            && value.bytes().all(ascii_id)
            && !value.starts_with('-')
            && !value.ends_with('-')
            && !value.contains("--");
        ensure!(well_shaped, InvestigationError::InvalidId);
        Ok(Self(value.to_owned()))
    }

    /// Derives a stable id: a readable slug from the name, a hash for the rest.
    /// The slug keeps every rule `parse` enforces, so this constructs directly.
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

/// A saved investigation, version 1. Loading runs the byte cap, a lenient
/// format/version probe, a strict parse, and [`Self::validate`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvestigationDefinitionV1 {
    pub format: String,
    pub version: u32,
    pub id: InvestigationId,
    pub revision: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub sql: String,
    pub dialect: SqlDialect,
    pub connection: String,
    #[serde(default)]
    pub objects: Vec<String>,
    pub schema_fingerprint: Option<String>,
    pub created_unix_ms: i64,
    pub updated_unix_ms: i64,
}

impl InvestigationDefinitionV1 {
    /// Reads serialized bytes: byte cap first, then a lenient `format`/`version`
    /// probe, then the strict parse with full validation.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, InvestigationError> {
        if bytes.len() > MAX_DEFINITION_BYTES {
            return Err(InvestigationError::Oversize(bytes.len()));
        }
        #[derive(Deserialize)]
        #[rustfmt::skip]
        struct Probe { format: Option<String>, version: Option<u32> }
        let probe: Probe =
            serde_json::from_slice(bytes).map_err(|_| InvestigationError::Malformed)?;
        if probe.format.as_deref() != Some(INVESTIGATION_FORMAT) {
            return Err(InvestigationError::NotAnInvestigation);
        }
        if let Some(version) = probe.version.filter(|v| *v != INVESTIGATION_FORMAT_VERSION) {
            return Err(InvestigationError::UnsupportedVersion(version));
        }
        let definition: Self =
            serde_json::from_slice(bytes).map_err(|_| InvestigationError::Malformed)?;
        definition.validate()?;
        Ok(definition)
    }

    /// Serializes after re-validating; the output obeys the same byte cap as
    /// the input.
    pub fn to_json_pretty(&self) -> Result<String, InvestigationError> {
        self.validate()?;
        let json = serde_json::to_string_pretty(self).map_err(|_| InvestigationError::Malformed)?;
        if json.len() > MAX_DEFINITION_BYTES {
            return Err(InvestigationError::Oversize(json.len()));
        }
        Ok(json)
    }

    /// Re-checks every bound; definitions that arrived as JSON skip no gate
    /// this way, and code-built definitions validate before storing too.
    pub fn validate(&self) -> Result<(), InvestigationError> {
        use self::InvestigationError::*;
        let objects = &self.objects;
        ensure!(self.format == INVESTIGATION_FORMAT, NotAnInvestigation);
        ensure!(
            self.version == INVESTIGATION_FORMAT_VERSION,
            UnsupportedVersion(self.version)
        );
        ensure!(self.revision >= 1, InvalidRevision);
        ensure!(!self.name.trim().is_empty(), InvalidName);
        ensure!(self.name.chars().count() <= MAX_NAME_CHARS, InvalidName);
        ensure!(!self.name.chars().any(char::is_control), ControlCharacter);
        if let Some(text) = &self.description {
            ensure!(text.len() <= MAX_DESCRIPTION_BYTES, DescriptionTooLong);
            ensure!(
                !text.chars().any(|c| c.is_control() && c != '\n'),
                ControlCharacter
            );
        }
        ensure!(!self.sql.trim().is_empty(), InvalidSql);
        ensure!(self.sql.len() <= MAX_SQL_BYTES, InvalidSql);
        let conn = &self.connection;
        let charset_ok = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-');
        ensure!(!conn.is_empty(), InvalidConnection);
        ensure!(conn.len() <= MAX_CONNECTION_CHARS, InvalidConnection);
        ensure!(conn.bytes().all(charset_ok), InvalidConnection);
        ensure!(objects.len() <= MAX_OBJECTS, TooManyObjects(objects.len()));
        for o in objects {
            ensure!(!o.is_empty() && o.len() <= MAX_OBJECT_BYTES, InvalidObject);
            ensure!(!o.chars().any(char::is_control), ControlCharacter);
        }
        let mut seen = HashSet::new();
        ensure!(objects.iter().all(|o| seen.insert(o)), DuplicateObject);
        if let Some(fp) = &self.schema_fingerprint {
            ensure!(!fp.is_empty(), InvalidFingerprint);
            ensure!(fp.len() <= MAX_FINGERPRINT_BYTES, InvalidFingerprint);
            ensure!(!fp.chars().any(char::is_control), InvalidFingerprint);
        }
        let (created, updated) = (self.created_unix_ms, self.updated_unix_ms);
        ensure!(updated >= created, UpdatedBeforeCreated);
        Ok(())
    }

    /// Records an edit as a new revision: revision and `updated_unix_ms`
    /// advance, the id and creation time stay, the new text replaces the old.
    /// The result is only as trustworthy as its inputs; validate before persisting.
    pub fn new_revision(
        &self,
        sql: impl Into<String>,
        name: impl Into<String>,
        description: Option<String>,
        now_unix_ms: i64,
    ) -> Self {
        Self {
            revision: self.revision.saturating_add(1),
            name: name.into(),
            description,
            sql: sql.into(),
            updated_unix_ms: now_unix_ms,
            ..self.clone()
        }
    }
}

#[cfg(test)]
#[path = "investigation_tests.rs"]
mod tests;
