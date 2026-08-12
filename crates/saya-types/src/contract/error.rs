use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ContractError {
    #[error("profile identity is not a valid opaque identifier")]
    InvalidProfileIdentity,

    #[error("claim id is not a valid identifier")]
    InvalidClaimId,

    #[error("database object name must not be empty")]
    EmptyName,

    #[error("database object name is too long")]
    NameTooLong,

    #[error("value contains control characters")]
    ControlCharacter,

    #[error("claim text must not be empty")]
    EmptyText,

    #[error("claim text is too long")]
    TextTooLong,

    #[error("too many referenced columns")]
    TooManyColumns,

    #[error("relationship column lists must have equal length")]
    ColumnCountMismatch,

    #[error("relationship must reference at least one column")]
    EmptyColumns,

    #[error("schema fingerprint digest is malformed")]
    InvalidFingerprint,
}
