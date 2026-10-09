/// A compacted session resumes with its summary and the verbatim tail: the
/// next request replays the last two turns beside the summary instead of the
/// whole transcript.
#[test]
fn a_compacted_session_resumes_with_summary_and_verbatim_tail() {
    let root = store_root("summary");
    let mut state = turns(6);
    compact(&mut state, SUMMARY);
    let resumed = save_and_resume(&root, &state);
    assert_eq!(
        resumed.compaction_summary.as_deref(),
        Some(SUMMARY),
        "the narrative is restored as the live summary"
    );
    assert_eq!(
        resumed.compacted_turns, 4,
        "four turns sit behind the summary"
    );
    assert_eq!(
        resumed.compaction_summary_text().as_deref(),
        Some(SUMMARY),
        "the permissive classification keeps the gate open"
    );
    let replayed = resumed.provider_request_history();
    assert_eq!(replayed.len(), 4, "only the tail replays: {replayed:?}");
    assert_eq!(replayed[0].role, "user");
    assert_eq!(replayed[0].content, "q4");
    assert_eq!(replayed[1].role, "assistant");
    assert_eq!(replayed[1].content, "a4");
    assert_eq!(replayed[2].content, "q5");
    assert_eq!(replayed[3].content, "a5");
    let mut again = resumed.clone();
    assert!(
        matches!(
            restore_continuation(&mut again),
            RestoreOutcome::Restored { compacted_turns: 4 }
        ),
        "the restored outcome names the boundary"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// Turns appended after the record was written do not invalidate the
/// restore: the prefix digest still matches, the boundary stays put, and the
/// request history carries the tail plus the new turns.
#[test]
fn turns_appended_after_compaction_keep_the_restore_valid() {
    let root = store_root("appended");
    let store = FsSessionStore::new(&root);
    let mut state = turns(6);
    compact(&mut state, SUMMARY);
    block_on(store.save(state.redacted())).expect("the compaction record saves");
    state.record_turn("q6", "a6", false, Vec::new());
    state.record_turn("q7", "a7", false, Vec::new());
    block_on(store.save(state.redacted())).expect("the appended record saves");
    let loaded = block_on(store.load("s1")).expect("loads").expect("present");
    let resumed = state_from_redacted(loaded, &defaults());
    assert_eq!(
        resumed.compacted_turns, 4,
        "the boundary keeps the old prefix"
    );
    assert_eq!(resumed.compaction_summary.as_deref(), Some(SUMMARY));
    let replayed = resumed.provider_request_history();
    assert_eq!(
        replayed.len(),
        8,
        "the tail and the new turns replay: {replayed:?}"
    );
    for (index, expected) in ["q4", "a4", "q5", "a5", "q6", "a6", "q7", "a7"]
        .iter()
        .enumerate()
    {
        assert_eq!(replayed[index].content, *expected, "{replayed:?}");
    }
    let _ = std::fs::remove_dir_all(root);
}
