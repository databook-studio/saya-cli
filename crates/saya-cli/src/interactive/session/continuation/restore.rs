//! Restores a saved session's continuation on resume: a persisted `Current`
//! record is validated against what the session actually holds now, and only
//! one that still describes this session restores its generated narrative as
//! the live compaction state. Two shapes exist: a boundary over this
//! session's persisted turns (the compaction case), and a boundary of zero —
//! the rotation-carried case, where the narrative summarises nothing in this
//! session's turns because it rides in from a rotated parent. Anything that
//! does not validate is ignored — the session resumes exactly as it would
//! without the continuation, and a rejected record stays on the state
//! untouched (a later save keeps it; the next compaction rebuilds it).

use super::source_digest;
use crate::interactive::session::continuation::compaction::manual::COMPACT_KEEP_TAIL_TURNS;
use crate::interactive::session::state::SessionState;
use saya_store::StoredContinuation;
use saya_types::{GeneratedNarrative, SessionContinuationV1, redact};

/// What resuming a saved session did with its persisted continuation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RestoreOutcome {
    /// The narrative was restored: `compacted_turns` older turns now sit
    /// behind the summary in the next request.
    Restored { compacted_turns: usize },
    /// The session carries no continuation: nothing to restore.
    NoContinuation,
    /// A `Current` continuation whose narrative was omitted at save time (a
    /// sensitive summary): the fact record stays, no summary is restored.
    NoNarrative,
    /// The continuation does not describe this session as it is now: the
    /// session resumes unchanged and the record stays on the state.
    Rejected(RejectReason),
}

/// Why a persisted continuation was not restored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RejectReason {
    /// A newer binary's record: never interpreted here.
    Unsupported,
    /// The typed record fails its own contract validation.
    Invalid,
    /// The persisted transcript prefix no longer matches the digest.
    TurnsChanged,
    /// The session's workspace root changed since the record was written.
    WorkspaceChanged,
    /// The session's profile changed since the record was written.
    ProfileChanged,
    /// The narrative is not safe to replay (the redactor would rewrite it).
    NarrativeUnsafe,
    /// The boundary does not fit the transcript: at or below the kept tail,
    /// or past the persisted turn count.
    BoundaryOutOfRange,
}

/// Validates the session's persisted continuation against the session as it
/// is now and, when it still describes this session, restores the generated
/// narrative as the live compaction state — `compaction_summary` plus
/// `compacted_turns`, so the next request carries the summary and the
/// verbatim tail instead of the whole transcript. A boundary of zero is the
/// rotation-carried case: the narrative summarises nothing in this session's
/// turns, so it restores with `compacted_turns` zero. The continuation is
/// never mutated: a rejected record stays exactly as it was loaded.
pub(crate) fn restore_continuation(state: &mut SessionState) -> RestoreOutcome {
    let Some(StoredContinuation::Current(record)) = state.continuation.as_ref() else {
        return if state.continuation.is_some() {
            RestoreOutcome::Rejected(RejectReason::Unsupported)
        } else {
            RestoreOutcome::NoContinuation
        };
    };
    if record.validate().is_err() {
        return RestoreOutcome::Rejected(RejectReason::Invalid);
    }
    let Some(narrative) = record.narrative.as_ref() else {
        return RestoreOutcome::NoNarrative;
    };
    // A boundary of zero carries the parent's narrative across a rotation:
    // it must digest the empty prefix — anything else does not name a
    // carried summary — and still describe this session's workspace and
    // profile, with the text replaying exactly as persisted.
    if record.source.through_turn == 0 {
        if source_digest(&[]) != record.source.source_sha256 {
            return RestoreOutcome::Rejected(RejectReason::TurnsChanged);
        }
        if let Some(reason) = common_checks(state, record, narrative) {
            return RestoreOutcome::Rejected(reason);
        }
        state.compaction_summary = Some(narrative.text.clone());
        state.compacted_turns = 0;
        return RestoreOutcome::Restored { compacted_turns: 0 };
    }
    // The boundary is positional over the persisted turns: it must name a
    // summarised region that exists and holds at least one turn older than
    // the kept tail — the shape every compaction produces (`COMPACT_MIN_
    // TURNS` guarantees at least one summarised turn).
    let Ok(through) = usize::try_from(record.source.through_turn) else {
        return RestoreOutcome::Rejected(RejectReason::BoundaryOutOfRange);
    };
    if through <= COMPACT_KEEP_TAIL_TURNS || through > state.turns.len() {
        return RestoreOutcome::Rejected(RejectReason::BoundaryOutOfRange);
    }
    if source_digest(&state.turns[..through]) != record.source.source_sha256 {
        return RestoreOutcome::Rejected(RejectReason::TurnsChanged);
    }
    if let Some(reason) = common_checks(state, record, narrative) {
        return RestoreOutcome::Rejected(reason);
    }
    state.compaction_summary = Some(narrative.text.clone());
    state.compacted_turns = through - COMPACT_KEEP_TAIL_TURNS;
    RestoreOutcome::Restored {
        compacted_turns: state.compacted_turns,
    }
}

/// The checks every restore shares, whatever the boundary: the record must
/// still describe this session's workspace and profile, and the narrative
/// must replay exactly as it was persisted.
fn common_checks(
    state: &SessionState,
    record: &SessionContinuationV1,
    narrative: &GeneratedNarrative,
) -> Option<RejectReason> {
    if record.source.workspace_root != state.workspace_root {
        return Some(RejectReason::WorkspaceChanged);
    }
    if record.source.profile != state.profile {
        return Some(RejectReason::ProfileChanged);
    }
    // Defense in depth: the saved narrative was redaction-stable by
    // construction; replaying it is refused if it no longer is.
    let text = narrative.text.as_str();
    if text.is_empty() || redact(text) != text {
        return Some(RejectReason::NarrativeUnsafe);
    }
    None
}

#[cfg(test)]
#[path = "restore_tests.rs"]
mod restore_tests;
