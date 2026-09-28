//! Bound enforcement for portable context documents: why a document is
//! refused, and the checks it must pass however it arrived.

use thiserror::Error;

use super::{
    CONTEXT_FORMAT, CONTEXT_FORMAT_VERSION, ContextDocumentV1, ContextItem, MAX_DOCUMENT_BYTES,
    MAX_ITEM_BYTES, MAX_ITEMS, MAX_ORIGIN_NOTE_BYTES, PortableObject,
};
use crate::MAX_NAME_CHARS;
use crate::contract::claim_payload::ClaimPayload;
use crate::contract::error::ContractError;
use crate::contract::identity::validate_name;

/// Why a context document was rejected: messages name the field and the bound
/// at fault; they never echo item text.
#[derive(Debug, Clone, PartialEq, Error)]
#[non_exhaustive]
pub enum ContextError {
    #[error("context document is {0} bytes, over the {MAX_DOCUMENT_BYTES}-byte limit")]
    Oversize(usize),
    #[error("unsupported context document version {0}")]
    UnsupportedVersion(u32),
    #[error("document is not a saya context document")]
    NotAContextDocument,
    #[error("context document is not valid JSON")]
    Malformed,
    #[error("too many items ({0})")]
    TooManyItems(usize),
    #[error("serialized item is {0} bytes, over the {MAX_ITEM_BYTES}-byte limit")]
    ItemOversize(usize),
    #[error("object names must be 1-{MAX_NAME_CHARS} characters")]
    InvalidObjectName,
    #[error("value contains control characters")]
    ControlCharacter,
    #[error("origin note must be at most {MAX_ORIGIN_NOTE_BYTES} bytes")]
    InvalidOriginNote,
    #[error(
        "relationship claims bind a profile identity and cannot travel in a portable context document"
    )]
    PayloadNotPortable,
}

impl ContextDocumentV1 {
    /// Re-checks every bound for a document that may have arrived as JSON,
    /// which skips no gate; code that builds or edits a document validates the
    /// same way before storing or sending it.
    pub fn validate(&self) -> Result<(), ContextError> {
        if self.format != CONTEXT_FORMAT {
            return Err(ContextError::NotAContextDocument);
        }
        if self.version != CONTEXT_FORMAT_VERSION {
            return Err(ContextError::UnsupportedVersion(self.version));
        }
        if self.items.len() > MAX_ITEMS {
            return Err(ContextError::TooManyItems(self.items.len()));
        }
        for item in &self.items {
            item.validate()?;
        }
        Ok(())
    }
}

impl ContextItem {
    /// Checks one item's bounds: the payload's portability, the object's
    /// names, the origin note, and the item's own serialized size.
    fn validate(&self) -> Result<(), ContextError> {
        if matches!(self.payload, ClaimPayload::Relationship { .. }) {
            return Err(ContextError::PayloadNotPortable);
        }
        self.object.validate()?;
        if let Some(note) = &self.origin_note {
            if note.len() > MAX_ORIGIN_NOTE_BYTES {
                return Err(ContextError::InvalidOriginNote);
            }
            if note.chars().any(char::is_control) {
                return Err(ContextError::ControlCharacter);
            }
        }
        let size = serde_json::to_vec(self).map_err(|_| ContextError::Malformed)?;
        if size.len() > MAX_ITEM_BYTES {
            return Err(ContextError::ItemOversize(size.len()));
        }
        Ok(())
    }
}

impl PortableObject {
    /// Names are validated exactly like `DatabaseObjectRef`'s fields, so an
    /// importer can build the ref without a second opinion about what fits.
    fn validate(&self) -> Result<(), ContextError> {
        for name in [
            self.catalog.as_deref(),
            self.schema.as_deref(),
            Some(&self.name),
        ]
        .into_iter()
        .flatten()
        {
            match validate_name(name) {
                Ok(()) => {}
                Err(ContractError::ControlCharacter) => {
                    return Err(ContextError::ControlCharacter);
                }
                Err(_) => return Err(ContextError::InvalidObjectName),
            }
        }
        Ok(())
    }
}
