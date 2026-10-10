use saya_types::PreferenceError;
use thiserror::Error;

use crate::coding::child::ChildSpecError;

pub const MAX_CONTEXT_BYTES: usize = 32 * 1024;
pub const MAX_CONTEXT_ITEMS: usize = 32;
pub const MAX_CHILD_TASK_BYTES: usize = 8 * 1024;
pub const MAX_CHILD_DEPENDENCIES: usize = 8;
pub const MAX_CHILD_DEPENDENCY_SUMMARY_BYTES: usize = 1024;
pub const MAX_CHILD_ARTIFACT_REFERENCES: usize = 16;
/// Combined instructions, skips, preferences, dependencies, and artifact refs per brief.
pub const MAX_CONTEXT_SOURCE_FACTS: usize = 80;
pub const MAX_CONTEXT_LABEL_BYTES: usize = 128;
pub const MAX_CONTEXT_REFERENCE_BYTES: usize = 128;
pub(super) const MAX_PROJECT_INSTRUCTION_BYTES: usize = 8 * 1024;
pub(super) const MAX_PERSONAL_INSTRUCTION_BYTES: usize = 8 * 1024;
pub(super) const MAX_INSTRUCTION_BYTES: usize = 16 * 1024;

/// Bounded failures while capturing original facts or preparing a fresh child request.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ChildContextError {
    #[error("original project path cannot be represented as canonical UTF-8")]
    OriginalProjectPathNotUtf8,
    #[error("original project identity could not be derived")]
    OriginalProjectIdentity,
    #[error("project preference belongs to a different worktree")]
    ForeignPreference,
    #[error("project preference record is invalid")]
    InvalidPreference(#[source] PreferenceError),
    #[error("project preference set is invalid")]
    InvalidPreferenceSet(#[source] PreferenceError),
    #[error("project preference count exceeds its established bound")]
    TooManyPreferences,
    #[error("instruction source digest is invalid")]
    InvalidInstructionDigest,
    #[error("original context contains too many source facts")]
    TooManySourceFacts,
    #[error("child task must be non-empty and within its byte bound")]
    TaskTooLarge,
    #[error("child dependency count exceeds its bound")]
    TooManyDependencies,
    #[error("child dependency identity is invalid or duplicated")]
    InvalidDependency,
    #[error("child dependency summary exceeds its byte bound")]
    DependencySummaryTooLarge,
    #[error("child artifact reference count exceeds its bound")]
    TooManyArtifacts,
    #[error("child artifact reference is invalid or duplicated")]
    InvalidArtifactReference,
    #[error("required child context exceeds its aggregate byte bound")]
    RequiredContextTooLarge,
    #[error("child context byte accounting overflowed")]
    ContextAccountingOverflow,
    #[error("child context contains too many items")]
    TooManyContextItems,
    #[error("child request failed existing local validation")]
    ChildSpec(#[from] ChildSpecError),
}

pub(super) fn safe_reference(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_CONTEXT_REFERENCE_BYTES
        && value != "."
        && value != ".."
        && value.is_ascii()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

pub(super) fn utf8_prefix_len(value: &str, maximum: usize) -> usize {
    let mut length = value.len().min(maximum);
    while !value.is_char_boundary(length) {
        length -= 1;
    }
    length
}

pub(super) fn add_bytes(total: usize, more: usize) -> Result<usize, ChildContextError> {
    total
        .checked_add(more)
        .ok_or(ChildContextError::ContextAccountingOverflow)
}
