//! Why a project preference record or set is refused.

use thiserror::Error;

use super::{
    MAX_PREFERENCE_RECALL_BYTES, MAX_PREFERENCE_RECORD_BYTES, MAX_PREFERENCE_TEXT_BYTES,
    MAX_PROJECT_PREFERENCES,
};

/// Why a project preference was rejected: messages name the field and the
/// bound at fault; they never echo preference text.
#[derive(Debug, Clone, PartialEq, Error)]
#[non_exhaustive]
pub enum PreferenceError {
    #[error("preference record is {0} bytes, over the {MAX_PREFERENCE_RECORD_BYTES}-byte limit")]
    Oversize(usize),
    #[error("unsupported project preference version {0}")]
    UnsupportedVersion(u32),
    #[error("preference record is not valid JSON")]
    Malformed,
    #[error("preference text must not be empty or whitespace")]
    TextEmpty,
    #[error("preference text is {0} bytes, over the {MAX_PREFERENCE_TEXT_BYTES}-byte limit")]
    TextOversize(usize),
    #[error("preference text contains control characters other than newline and tab")]
    TextControlCharacter,
    #[error("preference id must be `pp-` followed by 32 lowercase hex characters")]
    InvalidId,
    #[error("worktree id must be `wt-` followed by 64 lowercase hex characters")]
    InvalidWorktree,
    #[error("canonical worktree path must be non-empty and free of NUL bytes")]
    InvalidCanonicalPath,
    #[error("created_unix_ms must not be negative")]
    NegativeTimestamp,
    #[error("updated_unix_ms must not precede created_unix_ms")]
    TimestampOrder,
    #[error(
        "too many preferences ({0}, over the {limit} limit)",
        limit = MAX_PROJECT_PREFERENCES
    )]
    SetTooManyItems(usize),
    #[error("a preference set must be scoped to one canonical worktree")]
    MixedWorktrees,
    #[error("preference ids must be unique within a worktree")]
    DuplicateId,
    #[error(
        "aggregate recall text is {0} bytes, over the {MAX_PREFERENCE_RECALL_BYTES}-byte limit"
    )]
    RecallOversize(usize),
}
