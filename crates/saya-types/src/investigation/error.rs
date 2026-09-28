//! Why a saved investigation is refused.

use thiserror::Error;

use super::{
    MAX_CONNECTION_CHARS, MAX_DEFINITION_BYTES, MAX_FINGERPRINT_BYTES, MAX_NAME_CHARS,
    MAX_OBJECT_BYTES, MAX_SQL_BYTES,
};
use crate::params::ParamError;

/// Why a saved investigation was rejected: messages name the field and the
/// bound at fault; they never echo SQL or description content.
#[derive(Debug, Clone, PartialEq, Error)]
#[non_exhaustive]
pub enum InvestigationError {
    #[error("investigation document is {0} bytes, over the {MAX_DEFINITION_BYTES}-byte limit")]
    Oversize(usize),
    #[error("unsupported investigation version {0}")]
    UnsupportedVersion(u32),
    #[error("document is not a saya investigation")]
    NotAnInvestigation,
    #[error("investigation document is not valid JSON")]
    Malformed,
    #[error("investigation id is not a valid identifier")]
    InvalidId,
    #[error("investigation revision must be at least 1")]
    InvalidRevision,
    #[error("name must be 1-{MAX_NAME_CHARS} characters after trimming")]
    InvalidName,
    #[error("value contains control characters")]
    ControlCharacter,
    #[error("description is too long")]
    DescriptionTooLong,
    #[error("sql must be non-empty and at most {MAX_SQL_BYTES} bytes")]
    InvalidSql,
    #[error("connection must be 1-{MAX_CONNECTION_CHARS} characters of [A-Za-z0-9_.-]")]
    InvalidConnection,
    #[error("object names must be 1-{MAX_OBJECT_BYTES} bytes")]
    InvalidObject,
    #[error("too many objects ({0})")]
    TooManyObjects(usize),
    #[error("objects must not contain duplicates")]
    DuplicateObject,
    #[error("schema fingerprint must be 1-{MAX_FINGERPRINT_BYTES} printable bytes")]
    InvalidFingerprint,
    #[error("updated timestamp is before created timestamp")]
    UpdatedBeforeCreated,
    #[error("invalid parameter specification: {0}")]
    InvalidParameterSpec(#[from] ParamError),
}
