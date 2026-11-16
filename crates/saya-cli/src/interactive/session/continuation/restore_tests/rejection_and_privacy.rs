/// A rejected continuation is never dropped or rewritten: the state keeps
/// exactly what it loaded, `redacted()` hands it back, and the next save
/// persists it.
#[test]
fn a_rejected_continuation_is_never_dropped() {
    let root = store_root("kept");
    let mut state = turns(6);
    compact(&mut state, SUMMARY);
    let store = FsSessionStore::new(&root);
    block_on(store.save(state.redacted())).expect("saves");
    let mut loaded = block_on(store.load("s1")).expect("loads").expect("present");
    let expected = loaded.continuation.clone();
    loaded.turns[1].assistant = "edited a1".into();
    let mut resumed = state_from_redacted(loaded, &defaults());
    assert!(matches!(
        restore_continuation(&mut resumed),
        RestoreOutcome::Rejected(RejectReason::TurnsChanged)
    ));
    assert_eq!(
        resumed.continuation, expected,
        "the loaded record is untouched"
    );
    assert_eq!(
        resumed.redacted().continuation,
        expected,
        "redacted() hands it back to the store"
    );
    block_on(store.save(resumed.redacted())).expect("the next save keeps the rejected record");
    let reloaded = block_on(store.load("s1")).expect("loads").expect("present");
    assert_eq!(
        reloaded.continuation, expected,
        "the rejected record survives the save"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// The runtime privacy gate still holds after a restore: the summary is on
/// the session, but a cloud provider without data sharing never receives it.
#[test]
fn a_restored_summary_stays_behind_the_privacy_gate() {
    let root = store_root("privacy");
    let mut state = turns(5);
    state.provider = "openai".into();
    state.record_turn("q5", "a5", true, Vec::new());
    compact(&mut state, SUMMARY);
    let store = FsSessionStore::new(&root);
    block_on(store.save(state.redacted())).expect("saves");
    let loaded = block_on(store.load("s1")).expect("loads").expect("present");
    let resumed = state_from_redacted(
        loaded,
        &SessionDefaults {
            provider: "openai".into(),
            model: "m".into(),
            allow_data_sharing: false,
            approval_mode: "ask".into(),
        },
    );
    assert_eq!(
        resumed.compaction_summary.as_deref(),
        Some(SUMMARY),
        "the summary itself is restored; the gate is a request-time rule"
    );
    assert!(
        resumed.compaction_summary_text().is_none(),
        "a database-derived turn under a cloud provider keeps the gate shut"
    );
    let _ = std::fs::remove_dir_all(root);
}
