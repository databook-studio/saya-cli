//! The source boundary and the generated narrative: where the summarized
//! source ends and what the session wrote down about it — with the bound
//! checks that prose and digest must pass, however they arrived.

use serde::{Deserialize, Serialize};

use super::{ContinuationError, MAX_NARRATIVE_BYTES, MAX_USER_NOTES_BYTES};

/// Where the summarized source ends: the stable turn ordinal the session got
/// through — assigned at persist time, never a post-filter index — and a
/// digest over the redacted persisted turns it summarizes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceBoundary {
    pub through_turn: u64,
    /// A sha256 over the redacted persisted turns this continuation
    /// summarizes: it detects source drift, it does not authenticate the
    /// source.
    pub source_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
}

/// A generated wrap-up: bounded text and where it came from. User-authored
/// notes live in a separate field and are never merged into this.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeneratedNarrative {
    pub text: String,
    pub source: NarrativeSource,
}

/// What produced the narrative: the bounded summarizer or a deterministic
/// composer. No other source exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum NarrativeSource {
    Summarizer,
    Deterministic,
}

/// Free text may carry the control characters a rendered view already
/// understands — newline and tab — and no other.
fn prose_control_safe(text: &str) -> bool {
    text.chars()
        .all(|c| !c.is_control() || c == '\n' || c == '\t')
}

/// The checks a generated narrative must pass, however it arrived: present
/// means present with usable text, within [`MAX_NARRATIVE_BYTES`].
pub(super) fn validate_narrative(narrative: &GeneratedNarrative) -> Result<(), ContinuationError> {
    let text = &narrative.text;
    if text.is_empty() {
        return Err(ContinuationError::NarrativeEmpty);
    }
    if text.len() > MAX_NARRATIVE_BYTES {
        return Err(ContinuationError::NarrativeOversize(text.len()));
    }
    if !prose_control_safe(text) {
        return Err(ContinuationError::NarrativeControlCharacter);
    }
    Ok(())
}

/// The checks user notes must pass, however they arrived: usable text,
/// within [`MAX_USER_NOTES_BYTES`], apart from the narrative as always.
pub(super) fn validate_user_notes(notes: &str) -> Result<(), ContinuationError> {
    if notes.is_empty() {
        return Err(ContinuationError::UserNotesEmpty);
    }
    if notes.len() > MAX_USER_NOTES_BYTES {
        return Err(ContinuationError::UserNotesOversize(notes.len()));
    }
    if !prose_control_safe(notes) {
        return Err(ContinuationError::UserNotesControlCharacter);
    }
    Ok(())
}
