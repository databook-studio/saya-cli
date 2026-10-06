//! The pure rotation successor: derives the successor [`SessionState`] from
//! a parent that already holds a validated, compacted continuation, so a
//! session near the record cap can hand its work to a small record. D08b
//! wires this and removes the gate.
//!
//! What a successor carries: the parent's narrative as its compaction
//! summary, the user notes and deterministic file/check facts, the verbatim
//! not-yet-summarised tail, and the configuration the parent was opened
//! with. What it never carries: grants, approvals, or denials — the parent
//! link is lineage only — and no live outcome status, observations, or
//! token usage.

use crate::interactive::session_continuation::source_digest;
use crate::interactive::session_state::SessionState;
use saya_store::{ParentLink, StoredContinuation};
use saya_types::{CONTINUATION_VERSION, GeneratedNarrative, SessionContinuationV1, SourceBoundary};

/// Why a rotation refuses to build a successor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RotateRefusal {
    /// No typed continuation at all, or the live state holds no compacted
    /// turns to slice: nothing to carry over.
    NothingCompacted,
    /// The record's narrative was withheld (it looked sensitive), so a
    /// successor would start without context.
    NoNarrative,
    /// A newer binary's continuation: never interpreted here.
    Unsupported,
    /// The typed record fails its own contract validation.
    InvalidContinuation,
}

impl RotateRefusal {
    /// The user-facing sentence for the refusal.
    pub(crate) fn sentence(self) -> &'static str {
        match self {
            Self::NothingCompacted => {
                "Nothing to carry over yet: run /compact first so the session has a summary."
            }
            Self::NoNarrative => {
                "The saved summary was withheld (it looked sensitive), so a successor \
                 would start without context. Rotation was not done."
            }
            Self::Unsupported => {
                "This session was written by a newer version; it cannot be rotated here."
            }
            Self::InvalidContinuation => {
                "The saved summary no longer validates; run /compact again, then rotate."
            }
        }
    }
}

/// Builds the successor state for a rotation of `parent`, named `successor_id`
/// and linked back through `rotation_id`. The parent is never mutated; the
/// successor's continuation is a fresh boundary-zero record carrying the
/// parent's narrative, notes, and facts, validated before it is attached.
pub(crate) fn rotate_state(
    parent: &SessionState,
    successor_id: &str,
    rotation_id: &str,
) -> Result<SessionState, RotateRefusal> {
    let Some(StoredContinuation::Current(record)) = parent.continuation.as_ref() else {
        return Err(if parent.continuation.is_some() {
            RotateRefusal::Unsupported
        } else {
            RotateRefusal::NothingCompacted
        });
    };
    record
        .validate()
        .map_err(|_| RotateRefusal::InvalidContinuation)?;
    let Some(narrative) = record.narrative.as_ref() else {
        return Err(RotateRefusal::NoNarrative);
    };
    // The continuation alone does not make a rotation: the live state must
    // actually be compacted, and the compacted region must fit the
    // transcript — a resumed-and-rejected session has nothing to slice.
    if parent.compacted_turns == 0 || parent.compacted_turns > parent.turns.len() {
        return Err(RotateRefusal::NothingCompacted);
    }
    let carried = carried_record(parent, record, narrative);
    carried
        .validate()
        .map_err(|_| RotateRefusal::InvalidContinuation)?;
    let mut successor = build_successor(parent, successor_id, rotation_id, narrative);
    successor.continuation = Some(StoredContinuation::Current(carried));
    Ok(successor)
}

/// The successor's continuation: the parent's narrative, notes, and facts
/// under a boundary of zero — this narrative summarises nothing in THIS
/// session's turns, it is carried from the parent. The digest over the empty
/// prefix is the same function restore checks, so the carried case validates
/// by construction.
fn carried_record(
    parent: &SessionState,
    record: &SessionContinuationV1,
    narrative: &GeneratedNarrative,
) -> SessionContinuationV1 {
    SessionContinuationV1 {
        version: CONTINUATION_VERSION,
        source: SourceBoundary {
            through_turn: 0,
            source_sha256: source_digest(&[]),
            workspace_root: parent.workspace_root.clone(),
            profile: parent.profile.clone(),
        },
        narrative: Some(narrative.clone()),
        user_notes: record.user_notes.clone(),
        files: record.files.clone(),
        checks: record.checks.clone(),
    }
}

/// Clones the parent and overrides only what a successor must not carry;
/// every other field — profile, endpoint, modes, workspace, task list —
/// rides across exactly as the parent holds it.
fn build_successor(
    parent: &SessionState,
    successor_id: &str,
    rotation_id: &str,
    narrative: &GeneratedNarrative,
) -> SessionState {
    let mut successor = parent.clone();
    successor.id = successor_id.to_owned();
    successor.turns = parent.turns[parent.compacted_turns..].to_vec();
    successor.messages = Vec::new();
    successor.compaction_summary = Some(narrative.text.clone());
    successor.compacted_turns = 0;
    successor.parent = Some(ParentLink {
        id: parent.id.clone(),
        rotation_id: rotation_id.to_owned(),
    });
    successor.prior_tool_outcomes = None;
    successor.observations = Default::default();
    successor.usage = Default::default();
    successor.context_warned = false;
    successor.capacity_warned = false;
    successor.auto_compact_failed = false;
    successor.denied_programs = Default::default();
    successor
}

#[cfg(test)]
#[path = "session_rotate_tests.rs"]
mod session_rotate_tests;
