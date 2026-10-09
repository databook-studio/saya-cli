use super::*;

/// No continuation at all: nothing compacted, nothing to carry.
#[test]
fn an_uncompacted_session_refuses_rotation_as_nothing_compacted() {
    let state = turns(6);
    assert!(matches!(
        rotate_state(&state, "s2", "rot-1"),
        Err(RotateRefusal::NothingCompacted)
    ));
}

/// A newer binary's continuation is never interpreted, so it cannot be
/// rotated either.
#[test]
fn an_unsupported_continuation_refuses_rotation_as_unsupported() {
    let mut state = turns(6);
    state.continuation = Some(StoredContinuation::Unsupported(serde_json::json!({
        "version": 2, "future": true
    })));
    assert!(matches!(
        rotate_state(&state, "s2", "rot-1"),
        Err(RotateRefusal::Unsupported)
    ));
}

/// The record's narrative was withheld at save time: a successor would start
/// without context, so rotation refuses.
#[test]
fn a_narrative_less_record_refuses_rotation_as_no_narrative() {
    let mut state = compacted_parent();
    if let Some(StoredContinuation::Current(record)) = state.continuation.as_mut() {
        record.narrative = None;
    }
    assert!(matches!(
        rotate_state(&state, "s2", "rot-1"),
        Err(RotateRefusal::NoNarrative)
    ));
}

/// A valid typed record with a narrative on a session whose live state is
/// not compacted — e.g. just resumed with the record rejected — has nothing
/// to slice: refusal, not a whole-transcript successor.
#[test]
fn a_not_actually_compacted_session_refuses_rotation_as_nothing_compacted() {
    let mut state = turns(6);
    let record = SessionContinuationV1 {
        version: CONTINUATION_VERSION,
        source: SourceBoundary {
            through_turn: 6,
            source_sha256: source_digest(&state.turns),
            workspace_root: None,
            profile: None,
        },
        narrative: Some(GeneratedNarrative {
            text: "a carried-looking summary".into(),
            source: NarrativeSource::Summarizer,
        }),
        user_notes: None,
        files: Vec::new(),
        checks: Vec::new(),
    };
    record
        .validate()
        .expect("the hand-built record passes the contract");
    state.continuation = Some(StoredContinuation::Current(record));
    assert!(matches!(
        rotate_state(&state, "s2", "rot-1"),
        Err(RotateRefusal::NothingCompacted)
    ));
}

/// A record that fails its own contract cannot seed a successor.
#[test]
fn an_invalid_record_refuses_rotation_as_invalid_continuation() {
    let mut state = compacted_parent();
    if let Some(StoredContinuation::Current(record)) = state.continuation.as_mut() {
        record.source.source_sha256 = "nope".into();
    }
    assert!(matches!(
        rotate_state(&state, "s2", "rot-1"),
        Err(RotateRefusal::InvalidContinuation)
    ));
}

/// Every refusal names its exact user-facing sentence.
#[test]
fn every_refusal_names_its_sentence() {
    assert_eq!(
        RotateRefusal::NothingCompacted.sentence(),
        "Nothing to carry over yet: run /compact first so the session has a summary."
    );
    assert_eq!(
        RotateRefusal::NoNarrative.sentence(),
        "The saved summary was withheld (it looked sensitive), so a successor would \
         start without context. Rotation was not done."
    );
    assert_eq!(
        RotateRefusal::Unsupported.sentence(),
        "This session was written by a newer version; it cannot be rotated here."
    );
    assert_eq!(
        RotateRefusal::InvalidContinuation.sentence(),
        "The saved summary no longer validates; run /compact again, then rotate."
    );
}
