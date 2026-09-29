//! The portable reviewed-business-context contract: `saya.context` v1.
//!
//! One versioned, bounded document carrying confirmed business context —
//! descriptions, aliases, grains, roles, metrics, relationships, join rules,
//! notes — between profiles as a file. It carries what a claim *is*, never
//! who reviewed it or which machine it came from: no profile identity, no
//! review state, no evidence, no credentials, and no timestamp beyond
//! `exported_unix_ms`.

mod convert;
mod error;
mod payload;
mod validate;

use serde::{Deserialize, Serialize};

use crate::contract::DatabaseObjectKind;

pub use error::ContextError;
pub use payload::PortablePayload;

/// The `format` tag every context document carries, and the only version this
/// contract reads: anything else is refused, never downgraded.
pub const CONTEXT_FORMAT: &str = "saya.context";
pub const CONTEXT_FORMAT_VERSION: u32 = 1;
/// Document and collection bounds: a context file is at most 1 MiB, carries at
/// most [`MAX_ITEMS`] items, and each item is at most [`MAX_ITEM_BYTES`] of
/// serialized JSON; an origin note is at most [`MAX_ORIGIN_NOTE_BYTES`] bytes.
pub const MAX_DOCUMENT_BYTES: usize = 1024 * 1024;
pub const MAX_ITEMS: usize = 500;
pub const MAX_ITEM_BYTES: usize = 4 * 1024;
pub const MAX_ORIGIN_NOTE_BYTES: usize = 256;

/// A portable context document, version 1: the reviewed business context of
/// one profile, serialized as one portable file. Loading runs the byte cap, a
/// lenient format/version probe, a strict parse, and [`Self::validate`] — no
/// path skips a gate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextDocumentV1 {
    pub format: String,
    pub version: u32,
    pub exported_unix_ms: i64,
    pub items: Vec<ContextItem>,
}

/// One carried claim: the logical object it is about, the payload in portable
/// form (see [`PortablePayload`]), and an optional provenance note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextItem {
    pub object: PortableObject,
    pub payload: PortablePayload,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_note: Option<String>,
}

/// The logical object a claim is about, stripped of any profile identity:
/// catalog and schema are optional so the importer resolves them against the
/// destination profile's schema, and the name is bounded like an object ref.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortableObject {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub name: String,
    pub kind: DatabaseObjectKind,
}

impl ContextDocumentV1 {
    /// Reads a document from serialized bytes: the byte cap before any
    /// parsing, then a lenient probe of only `format`/`version` so an unknown
    /// version or a foreign document fails precisely, then the strict parse
    /// with every bound re-checked.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, ContextError> {
        if bytes.len() > MAX_DOCUMENT_BYTES {
            return Err(ContextError::Oversize(bytes.len()));
        }
        #[derive(Deserialize)]
        struct Probe {
            format: Option<String>,
            version: Option<u32>,
        }
        let probe: Probe = serde_json::from_slice(bytes).map_err(|_| ContextError::Malformed)?;
        if probe.format.as_deref() != Some(CONTEXT_FORMAT) {
            return Err(ContextError::NotAContextDocument);
        }
        if let Some(version) = probe.version.filter(|v| *v != CONTEXT_FORMAT_VERSION) {
            return Err(ContextError::UnsupportedVersion(version));
        }
        let document: Self = serde_json::from_slice(bytes).map_err(|_| ContextError::Malformed)?;
        document.validate()?;
        Ok(document)
    }

    /// Serializes after re-validating, so a document mutated in code cannot
    /// leave this crate in a shape it would refuse on the way back in. The
    /// output obeys the same byte cap as the input.
    pub fn to_json_pretty(&self) -> Result<String, ContextError> {
        self.validate()?;
        let json = serde_json::to_string_pretty(self).map_err(|_| ContextError::Malformed)?;
        if json.len() > MAX_DOCUMENT_BYTES {
            return Err(ContextError::Oversize(json.len()));
        }
        Ok(json)
    }
}

#[cfg(test)]
mod tests;
