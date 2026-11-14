//! Why a portable context document is refused.

use thiserror::Error;

use super::{MAX_DOCUMENT_BYTES, MAX_ITEM_BYTES, MAX_ORIGIN_NOTE_BYTES};
use crate::{MAX_NAME_CHARS, MAX_REFERENCED_COLUMNS, MAX_TEXT_CHARS};

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
        "a claim that names a second object must travel in its portable variant, not the claim wrapper"
    )]
    PayloadNotPortable,
    #[error(
        "column lists must pair positionally and hold at most {MAX_REFERENCED_COLUMNS} valid names"
    )]
    InvalidColumns,
    #[error("payload text must be 1-{MAX_TEXT_CHARS} characters without control characters")]
    InvalidText,
    #[error("claim target could not be resolved against this profile's schema")]
    UnresolvedTarget,
    #[error("claim payload does not satisfy its validators")]
    InvalidPayload,
}
