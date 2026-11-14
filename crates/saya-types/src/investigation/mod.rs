//! The saved-investigation contract: a portable, versioned definition of one
//! bounded, read-only SQL investigation — what the investigation *is* and
//! nothing about any run against it: no credentials, results, transcripts,
//! grants, or machine-specific profile identity.

mod error;
mod id;
mod validate;

use serde::{Deserialize, Serialize};

use crate::dialect::SqlDialect;
use crate::params::ParameterSpec;

pub use error::InvestigationError;
pub use id::InvestigationId;
#[cfg(test)]
use id::MAX_ID_CHARS;

/// The `format` tag every saved investigation carries, and the only version
/// this contract reads: anything else is refused, never downgraded.
pub const INVESTIGATION_FORMAT: &str = "saya.investigation";
pub const INVESTIGATION_FORMAT_VERSION: u32 = 1;
/// Document, field, and collection bounds; the name bound counts Unicode
/// scalar values, not bytes.
pub const MAX_DEFINITION_BYTES: usize = 128 * 1024;
pub const MAX_SQL_BYTES: usize = 64 * 1024;
pub const MAX_NAME_CHARS: usize = 80;
pub const MAX_DESCRIPTION_BYTES: usize = 2048;
pub const MAX_OBJECTS: usize = 256;
pub const MAX_OBJECT_BYTES: usize = 256;
pub const MAX_CONNECTION_CHARS: usize = 64;
pub const MAX_FINGERPRINT_BYTES: usize = 128;

/// A saved investigation, version 1: what the investigation is, serialized
/// as one portable document. Loading runs the byte cap, a lenient
/// format/version probe, a strict parse, and [`Self::validate`] — no path
/// skips a gate.
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
    /// Declared parameters for the `:name` placeholders in the SQL. Empty —
    /// and omitted from serialization — for parameter-free investigations;
    /// changing the list is a new revision.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<ParameterSpec>,
    pub dialect: SqlDialect,
    pub connection: String,
    #[serde(default)]
    pub objects: Vec<String>,
    pub schema_fingerprint: Option<String>,
    pub created_unix_ms: i64,
    pub updated_unix_ms: i64,
}

impl InvestigationDefinitionV1 {
    /// Reads a definition from serialized bytes: the byte cap before any
    /// parsing, then a lenient probe of only `format`/`version` so an
    /// unknown version or a foreign document fails precisely, then the
    /// strict parse with every bound re-checked.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, InvestigationError> {
        if bytes.len() > MAX_DEFINITION_BYTES {
            return Err(InvestigationError::Oversize(bytes.len()));
        }
        #[derive(Deserialize)]
        struct Probe {
            format: Option<String>,
            version: Option<u32>,
        }
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

    /// Serializes after re-validating, so a definition mutated in code cannot
    /// leave this crate in a shape it would refuse on the way back in. The
    /// output obeys the same byte cap as the input.
    pub fn to_json_pretty(&self) -> Result<String, InvestigationError> {
        self.validate()?;
        let json = serde_json::to_string_pretty(self).map_err(|_| InvestigationError::Malformed)?;
        if json.len() > MAX_DEFINITION_BYTES {
            return Err(InvestigationError::Oversize(json.len()));
        }
        Ok(json)
    }

    /// Records an edit as a new revision: revision and `updated_unix_ms`
    /// advance, the id and creation time stay, and the new text replaces the
    /// old. The result is only as trustworthy as its inputs; validate before
    /// persisting.
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
mod tests;
