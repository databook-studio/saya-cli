/// An edited persisted turn breaks the prefix digest: the continuation is
/// rejected and the session resumes with every turn replayed.
#[test]
fn a_changed_stored_turn_rejects_the_continuation_as_turns_changed() {
    let root = store_root("turns-changed");
    let mut state = turns(6);
    compact(&mut state, SUMMARY);
    let store = FsSessionStore::new(&root);
    block_on(store.save(state.redacted())).expect("saves");
    let mut loaded = block_on(store.load("s1")).expect("loads").expect("present");
    loaded.turns[0].user = "edited q0".into();
    let mut resumed = state_from_redacted(loaded, &defaults());
    assert!(matches!(
        restore_continuation(&mut resumed),
        RestoreOutcome::Rejected(RejectReason::TurnsChanged)
    ));
    assert_resumed_unchanged(&resumed, 6);
    let _ = std::fs::remove_dir_all(root);
}

/// A changed workspace root means the record no longer describes this
/// session: rejected, and the resumed session keeps the record's root.
#[test]
fn a_changed_workspace_rejects_the_continuation_as_workspace_changed() {
    let root = store_root("workspace-changed");
    let mut state = turns(6);
    state.workspace_root = Some("/projects/saya".into());
    compact(&mut state, SUMMARY);
    let store = FsSessionStore::new(&root);
    block_on(store.save(state.redacted())).expect("saves");
    let mut loaded = block_on(store.load("s1")).expect("loads").expect("present");
    loaded.workspace_root = Some("/elsewhere".into());
    let mut resumed = state_from_redacted(loaded, &defaults());
    assert!(matches!(
        restore_continuation(&mut resumed),
        RestoreOutcome::Rejected(RejectReason::WorkspaceChanged)
    ));
    assert_resumed_unchanged(&resumed, 6);
    assert_eq!(
        resumed.workspace_root.as_deref(),
        Some("/elsewhere"),
        "the resumed session keeps the record's root"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// A changed profile means the record no longer describes this session.
#[test]
fn a_changed_profile_rejects_the_continuation_as_profile_changed() {
    let root = store_root("profile-changed");
    let mut state = turns(6);
    state.profile = Some("analytics".into());
    compact(&mut state, SUMMARY);
    let store = FsSessionStore::new(&root);
    block_on(store.save(state.redacted())).expect("saves");
    let mut loaded = block_on(store.load("s1")).expect("loads").expect("present");
    loaded.profile = Some("other".into());
    let mut resumed = state_from_redacted(loaded, &defaults());
    assert!(matches!(
        restore_continuation(&mut resumed),
        RestoreOutcome::Rejected(RejectReason::ProfileChanged)
    ));
    assert_resumed_unchanged(&resumed, 6);
    assert_eq!(
        resumed.profile.as_deref(),
        Some("other"),
        "the resumed session keeps the record's profile"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// A boundary past the persisted turn count cannot name this transcript.
#[test]
fn a_truncated_transcript_rejects_the_continuation_as_boundary_out_of_range() {
    let root = store_root("boundary");
    let mut state = turns(6);
    compact(&mut state, SUMMARY);
    let store = FsSessionStore::new(&root);
    block_on(store.save(state.redacted())).expect("saves");
    let mut loaded = block_on(store.load("s1")).expect("loads").expect("present");
    loaded.turns.truncate(3);
    let mut resumed = state_from_redacted(loaded, &defaults());
    assert!(matches!(
        restore_continuation(&mut resumed),
        RestoreOutcome::Rejected(RejectReason::BoundaryOutOfRange)
    ));
    assert_resumed_unchanged(&resumed, 3);
    let _ = std::fs::remove_dir_all(root);
}

/// A narrative the redactor would rewrite is refused at restore time —
/// defense in depth beside the save-time omission.
#[test]
fn an_unsafe_narrative_is_rejected_as_narrative_unsafe() {
    let mut state = turns(6);
    let text = "password=hunter2";
    assert_ne!(
        redact(text),
        text,
        "the test needs a redactor-rewritten narrative"
    );
    let record = SessionContinuationV1 {
        version: CONTINUATION_VERSION,
        source: SourceBoundary {
            through_turn: 6,
            source_sha256: source_digest(&state.turns),
            workspace_root: state.workspace_root.clone(),
            profile: state.profile.clone(),
        },
        narrative: Some(GeneratedNarrative {
            text: text.into(),
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
        restore_continuation(&mut state),
        RestoreOutcome::Rejected(RejectReason::NarrativeUnsafe)
    ));
    assert_resumed_unchanged(&state, 6);
}

/// A newer binary's continuation is never interpreted: it is rejected and
/// the value stays untouched.
#[test]
fn an_unsupported_continuation_is_rejected_and_untouched() {
    let mut state = turns(6);
    let raw = serde_json::json!({"version": 2, "future": true});
    state.continuation = Some(StoredContinuation::Unsupported(raw.clone()));
    assert!(matches!(
        restore_continuation(&mut state),
        RestoreOutcome::Rejected(RejectReason::Unsupported)
    ));
    assert_eq!(
        state.continuation,
        Some(StoredContinuation::Unsupported(raw)),
        "the value is untouched"
    );
    assert_resumed_unchanged(&state, 6);
}

/// A session without a continuation has nothing to restore.
#[test]
fn a_session_without_a_continuation_resumes_without_one() {
    let mut state = turns(6);
    assert!(matches!(
        restore_continuation(&mut state),
        RestoreOutcome::NoContinuation
    ));
    assert_resumed_unchanged(&state, 6);
}

/// A summary the redactor rewrites was omitted from the saved record; the
/// resume restores no summary but keeps the fact record on the state.
#[test]
fn a_sensitive_narrative_omitted_at_save_resumes_without_a_summary() {
    let root = store_root("no-narrative");
    let sensitive = "setup used password=hunter2 and finished the edit.";
    assert_ne!(redact(sensitive), sensitive);
    let mut state = turns(6);
    compact(&mut state, sensitive);
    let record = match &state.continuation {
        Some(StoredContinuation::Current(record)) => record.clone(),
        other => panic!("the compaction saved a typed record: {other:?}"),
    };
    assert!(
        record.narrative.is_none(),
        "the sensitive narrative was omitted at save"
    );
    let resumed = save_and_resume(&root, &state);
    let mut again = resumed.clone();
    assert!(matches!(
        restore_continuation(&mut again),
        RestoreOutcome::NoNarrative
    ));
    assert_resumed_unchanged(&resumed, 6);
    assert_eq!(
        resumed.continuation,
        Some(StoredContinuation::Current(record)),
        "the fact record stays on the resumed state"
    );
    let _ = std::fs::remove_dir_all(root);
}
