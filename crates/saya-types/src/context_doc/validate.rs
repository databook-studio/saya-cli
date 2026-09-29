//! Bound enforcement for portable context documents: the checks every
//! document must pass, however it arrived. The payload's own shape checks
//! live with the payload in `payload`.

use super::{
    CONTEXT_FORMAT, CONTEXT_FORMAT_VERSION, ContextDocumentV1, ContextError, ContextItem,
    MAX_ITEM_BYTES, MAX_ITEMS, MAX_ORIGIN_NOTE_BYTES, PortableObject,
};
use crate::contract::error::ContractError;
use crate::contract::identity::validate_name;

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
    /// Checks one item's bounds: the payload's shape, the object's names, the
    /// origin note, and the item's own serialized size.
    fn validate(&self) -> Result<(), ContextError> {
        self.payload.validate()?;
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
    pub(super) fn validate(&self) -> Result<(), ContextError> {
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
