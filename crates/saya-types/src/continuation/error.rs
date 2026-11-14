//! Why a session continuation record is refused.

use thiserror::Error;

use super::{
    MAX_CHECK_FACTS, MAX_CHECK_LABEL_CHARS, MAX_CONTINUATION_BYTES, MAX_FACT_PATH_BYTES,
    MAX_FILE_FACTS, MAX_NARRATIVE_BYTES, MAX_USER_NOTES_BYTES,
};

/// Why a continuation record was rejected: messages name the field and the
/// bound at fault; they never echo record text.
#[derive(Debug, Clone, PartialEq, Error)]
#[non_exhaustive]
pub enum ContinuationError {
    #[error("continuation record is {0} bytes, over the {MAX_CONTINUATION_BYTES}-byte limit")]
    Oversize(usize),
    #[error("unsupported continuation version {0}")]
    UnsupportedVersion(u32),
    #[error("continuation record is not valid JSON")]
    Malformed,
    #[error("narrative is {0} bytes, over the {MAX_NARRATIVE_BYTES}-byte limit")]
    NarrativeOversize(usize),
    #[error("narrative must not be empty")]
    NarrativeEmpty,
    #[error("narrative contains control characters other than newline and tab")]
    NarrativeControlCharacter,
    #[error("user notes are {0} bytes, over the {MAX_USER_NOTES_BYTES}-byte limit")]
    UserNotesOversize(usize),
    #[error("user notes must not be empty")]
    UserNotesEmpty,
    #[error("user notes contain control characters other than newline and tab")]
    UserNotesControlCharacter,
    #[error("too many file facts ({0}, over the {MAX_FILE_FACTS} limit)")]
    TooManyFiles(usize),
    #[error("too many check facts ({0}, over the {MAX_CHECK_FACTS} limit)")]
    TooManyChecks(usize),
    #[error(
        "fact path must be relative, at most {MAX_FACT_PATH_BYTES} bytes, without `..`, empty \
         components, or control characters"
    )]
    InvalidFactPath,
    #[error("fact path is {0} bytes, over the {MAX_FACT_PATH_BYTES}-byte limit")]
    FactPathOversize(usize),
    #[error("digest must be 64 lowercase hex characters")]
    InvalidDigest,
    #[error("check label must be 1-{MAX_CHECK_LABEL_CHARS} characters without control characters")]
    InvalidCheckLabel,
    #[error("tool name is not name-shaped")]
    InvalidToolName,
    #[error("observed_unix_ms must not be negative")]
    NegativeTimestamp,
    #[error("file fact paths must be unique")]
    DuplicateFilePath,
}
