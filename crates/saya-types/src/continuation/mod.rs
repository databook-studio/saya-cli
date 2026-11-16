//! The session continuation contract: a bounded, versioned record a finished
//! session may carry so the next one can pick up where it stopped.
//!
//! Pure contract — no I/O, no persistence, no grants. The record carries the
//! source boundary it summarizes, a generated narrative, separately-preserved
//! user notes, and deterministic file/check facts, each with tool provenance.
//! It binds no authority: it describes what happened, and permissions are
//! always freshly resolved under the rules in force when a resumed session
//! acts. Absence stays representable — legacy sessions carry no continuation.

mod boundary;
mod error;
mod facts;
mod validate;

use serde::{Deserialize, Serialize};

use crate::MAX_TASK_TITLE_CHARS;

pub use boundary::{GeneratedNarrative, NarrativeSource, SourceBoundary};
pub use error::ContinuationError;
pub use facts::{CheckFact, CheckOutcome, FactProvenance, FileFact};

/// The only continuation version this contract reads: anything else is
/// refused, never downgraded.
pub const CONTINUATION_VERSION: u32 = 1;
/// Serialized bounds: the whole record is at most [`MAX_CONTINUATION_BYTES`]
/// of JSON, the generated narrative at most [`MAX_NARRATIVE_BYTES`] bytes,
/// and user notes at most [`MAX_USER_NOTES_BYTES`] bytes.
pub const MAX_CONTINUATION_BYTES: usize = 32 * 1024;
pub const MAX_NARRATIVE_BYTES: usize = 8 * 1024;
pub const MAX_USER_NOTES_BYTES: usize = 8 * 1024;
/// Fact bounds: at most [`MAX_FILE_FACTS`] file facts and [`MAX_CHECK_FACTS`]
/// check facts; a fact path is at most [`MAX_FACT_PATH_BYTES`] bytes; a check
/// label is bounded like a task title.
pub const MAX_FILE_FACTS: usize = 64;
pub const MAX_CHECK_FACTS: usize = 32;
pub const MAX_FACT_PATH_BYTES: usize = 512;
pub const MAX_CHECK_LABEL_CHARS: usize = MAX_TASK_TITLE_CHARS;

/// A session continuation, version 1: what the session got through, what it
/// wrote down, and the deterministic facts behind it. Loading runs the byte
/// cap, a lenient version probe, a strict parse, and [`Self::validate`] — no
/// path skips a gate. The record binds no authority: it carries no capability,
/// a digest certifies nothing, and permissions are always freshly resolved
/// under the rules in force when the resumed session acts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionContinuationV1 {
    pub version: u32,
    pub source: SourceBoundary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub narrative: Option<GeneratedNarrative>,
    /// User-authored notes, kept apart from the generated narrative and
    /// never merged into it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_notes: Option<String>,
    #[serde(default)]
    pub files: Vec<FileFact>,
    #[serde(default)]
    pub checks: Vec<CheckFact>,
}

impl SessionContinuationV1 {
    /// Reads a continuation from serialized bytes: the byte cap before any
    /// parsing, then a lenient probe of only `version` so an unknown version
    /// fails precisely, then the strict parse with every bound re-checked.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, ContinuationError> {
        if bytes.len() > MAX_CONTINUATION_BYTES {
            return Err(ContinuationError::Oversize(bytes.len()));
        }
        #[derive(Deserialize)]
        struct Probe {
            version: Option<u32>,
        }
        let probe: Probe =
            serde_json::from_slice(bytes).map_err(|_| ContinuationError::Malformed)?;
        if let Some(version) = probe.version.filter(|v| *v != CONTINUATION_VERSION) {
            return Err(ContinuationError::UnsupportedVersion(version));
        }
        let record: Self =
            serde_json::from_slice(bytes).map_err(|_| ContinuationError::Malformed)?;
        record.validate()?;
        Ok(record)
    }

    /// Serializes after re-validating, so a record mutated in code cannot
    /// leave this crate in a shape it would refuse on the way back in.
    pub fn to_json_bytes(&self) -> Result<Vec<u8>, ContinuationError> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|_| ContinuationError::Malformed)
    }
}
