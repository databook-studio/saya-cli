fn carried_record(
    digest: String,
    workspace: Option<String>,
    profile: Option<String>,
) -> SessionContinuationV1 {
    SessionContinuationV1 {
        version: CONTINUATION_VERSION,
        source: SourceBoundary {
            through_turn: 0,
            source_sha256: digest,
            workspace_root: workspace,
            profile,
        },
        narrative: Some(GeneratedNarrative {
            text: SUMMARY.into(),
            source: NarrativeSource::Summarizer,
        }),
        user_notes: None,
        files: Vec::new(),
        checks: Vec::new(),
    }
}

/// A boundary-zero record is the rotation-carried case: the narrative
/// summarises nothing in THIS session's turns, so it restores as a summary
/// over an empty compacted region and the record stays untouched.
///
/// A boundary-zero record restores as a carried summary: the digest over the
/// empty prefix matches, so the narrative rides in with `compacted_turns`
/// zero — it summarises nothing in this session's own turns.
#[test]
fn a_boundary_zero_record_restores_as_a_carried_summary() {
    let mut state = turns(2);
    let record = carried_record(source_digest(&[]), None, None);
    record
        .validate()
        .expect("the hand-built carried record passes the contract");
    state.continuation = Some(StoredContinuation::Current(record.clone()));
    assert!(matches!(
        restore_continuation(&mut state),
        RestoreOutcome::Restored { compacted_turns: 0 }
    ));
    assert_eq!(
        state.compaction_summary.as_deref(),
        Some(SUMMARY),
        "the carried narrative is restored as the live summary"
    );
    assert_eq!(
        state.compacted_turns, 0,
        "nothing in this session is behind it"
    );
    assert_eq!(
        state.continuation,
        Some(StoredContinuation::Current(record)),
        "the record stays untouched"
    );
}

/// A boundary-zero record whose digest is not the empty-prefix digest does
/// not describe a carried summary: rejected, record untouched, plain resume.
#[test]
fn a_carried_record_with_a_wrong_empty_digest_rejects_as_turns_changed() {
    let mut state = turns(2);
    let record = carried_record("f".repeat(64), None, None);
    state.continuation = Some(StoredContinuation::Current(record.clone()));
    assert!(matches!(
        restore_continuation(&mut state),
        RestoreOutcome::Rejected(RejectReason::TurnsChanged)
    ));
    assert_resumed_unchanged(&state, 2);
    assert_eq!(
        state.continuation,
        Some(StoredContinuation::Current(record)),
        "the record stays untouched"
    );
}

/// A boundary-zero record carries the workspace it was written under: a
/// different root means it no longer describes this session.
#[test]
fn a_carried_record_with_a_changed_workspace_rejects_as_workspace_changed() {
    let mut state = turns(2);
    state.workspace_root = Some("/projects/saya".into());
    let record = carried_record(source_digest(&[]), Some("/elsewhere".into()), None);
    state.continuation = Some(StoredContinuation::Current(record));
    assert!(matches!(
        restore_continuation(&mut state),
        RestoreOutcome::Rejected(RejectReason::WorkspaceChanged)
    ));
    assert_resumed_unchanged(&state, 2);
}

/// A boundary-zero record carries the profile it was written under: a
/// different profile means it no longer describes this session.
#[test]
fn a_carried_record_with_a_changed_profile_rejects_as_profile_changed() {
    let mut state = turns(2);
    state.profile = Some("analytics".into());
    let record = carried_record(source_digest(&[]), None, Some("other".into()));
    state.continuation = Some(StoredContinuation::Current(record));
    assert!(matches!(
        restore_continuation(&mut state),
        RestoreOutcome::Rejected(RejectReason::ProfileChanged)
    ));
    assert_resumed_unchanged(&state, 2);
}

/// The old boundary rule is unchanged for real boundaries: 1 and 2 sit at or
/// below the kept tail, so they stay out of range even when their digests
/// match their prefixes exactly.
#[test]
fn boundaries_1_and_2_still_reject_as_boundary_out_of_range() {
    for through in [1usize, 2] {
        let mut state = turns(6);
        let record = SessionContinuationV1 {
            version: CONTINUATION_VERSION,
            source: SourceBoundary {
                through_turn: through as u64,
                source_sha256: source_digest(&state.turns[..through]),
                workspace_root: None,
                profile: None,
            },
            narrative: Some(GeneratedNarrative {
                text: SUMMARY.into(),
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
        assert!(
            matches!(
                restore_continuation(&mut state),
                RestoreOutcome::Rejected(RejectReason::BoundaryOutOfRange)
            ),
            "through={through} stays out of range"
        );
        assert_resumed_unchanged(&state, 6);
    }
}
