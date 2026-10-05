//! Why a checkpoint record is refused, and why an edit is refused a
//! checkpoint.

use thiserror::Error;

use super::{
    MAX_CHECKPOINT_FILE_BYTES, MAX_CHECKPOINT_PATH_BYTES, MAX_CHECKPOINT_RECORD_BYTES,
    MAX_CHECKPOINT_SESSION_BYTES, MAX_CHECKPOINTS_PER_SESSION,
};

/// Why a checkpoint record was rejected: messages name the field and the
/// bound at fault; they never echo record text.
#[derive(Debug, Clone, PartialEq, Error)]
#[non_exhaustive]
pub enum CheckpointError {
    #[error("checkpoint record is {0} bytes, over the {MAX_CHECKPOINT_RECORD_BYTES}-byte limit")]
    Oversize(usize),
    #[error("unsupported checkpoint version {0}")]
    UnsupportedVersion(u32),
    #[error("checkpoint record is not valid JSON")]
    Malformed,
    #[error("checkpoint id must be `ck-` followed by 32 lowercase hex characters")]
    InvalidId,
    #[error(
        "checkpoint path must be relative, at most {MAX_CHECKPOINT_PATH_BYTES} bytes, without \
         `..`, empty components, or control characters"
    )]
    InvalidPath,
    #[error("checkpoint path is {0} bytes, over the {MAX_CHECKPOINT_PATH_BYTES}-byte limit")]
    PathOversize(usize),
    #[error("digest must be 64 lowercase hex characters")]
    InvalidDigest,
    #[error("recorded version is {0} bytes, over the {MAX_CHECKPOINT_FILE_BYTES}-byte limit")]
    VersionTooLarge(u64),
    #[error("a committed checkpoint requires a recorded after-image")]
    CommittedRequiresAfter,
    #[error("a prepared or uncertain checkpoint must not carry an after-image")]
    StateDisallowsAfter,
    #[error("created_unix_ms must not be negative")]
    NegativeTimestamp,
    #[error("updated_unix_ms must not precede created_unix_ms")]
    TimestampOrder,
}

/// Why an edit was refused a checkpoint before anything was mutated. A
/// refusal never evicts or rewrites existing history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum CheckpointRefusal {
    #[error("the edit's retained bytes exceed the {MAX_CHECKPOINT_FILE_BYTES}-byte limit")]
    FileTooLarge,
    #[error(
        "the session's retained bytes would exceed the {MAX_CHECKPOINT_SESSION_BYTES}-byte limit"
    )]
    SessionBytesExceeded,
    #[error("the session already holds {MAX_CHECKPOINTS_PER_SESSION} checkpoints")]
    TooManyCheckpoints,
}
