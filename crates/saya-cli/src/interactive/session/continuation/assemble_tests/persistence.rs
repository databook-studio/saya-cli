/// The parent lineage link rides the record the same way and round-trips.
#[test]
fn the_parent_link_round_trips() {
    let root = store_root("parent");
    let store = FsSessionStore::new(&root);
    let mut state = turns(2);
    state.parent = Some(ParentLink {
        id: "parent-1".into(),
        rotation_id: "rot-1".into(),
    });
    block_on(store.save(state.redacted())).expect("saves");
    let loaded = block_on(store.load("s1")).expect("loads").expect("present");
    let resumed = state_from_redacted(loaded, &defaults());
    assert_eq!(
        resumed.parent, state.parent,
        "the lineage link round-trips through the store and resume"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// A failed save leaves the last valid record on disk intact, and nothing in
/// memory claims the new continuation is durable: the state keeps it for a
/// retry, and once the oversize turn is removed the retry persists it.
#[test]
fn a_failed_save_keeps_the_previous_valid_record() {
    let root = store_root("failed-save");
    let store = FsSessionStore::new(&root);
    let mut state = turns(2);
    let first = assemble_continuation(&state, "first summary.", &digest_of).expect("assembles");
    state.continuation = Some(StoredContinuation::Current(first.clone()));
    block_on(store.save(state.redacted())).expect("the first record saves");

    let second = assemble_continuation(&state, "second summary.", &digest_of).expect("assembles");
    state.continuation = Some(StoredContinuation::Current(second.clone()));

    state.record_turn("x".repeat(MAX_SESSION_BYTES), "answer", false, Vec::new());
    let error =
        block_on(store.save(state.redacted())).expect_err("an oversize record must be refused");
    assert_eq!(error, StoreError::LimitExceeded);
    let loaded = block_on(store.load("s1")).expect("loads").expect("present");
    assert_eq!(
        loaded.continuation,
        Some(StoredContinuation::Current(first)),
        "the last valid record kept its continuation"
    );
    assert_eq!(
        state.continuation,
        Some(StoredContinuation::Current(second.clone())),
        "the state keeps the new continuation for a retry; no flag claims it durable"
    );

    state.turns.pop();
    state.messages.pop();
    state.messages.pop();
    block_on(store.save(state.redacted())).expect("the retry saves once the oversize turn is gone");
    let loaded = block_on(store.load("s1")).expect("loads").expect("present");
    assert_eq!(
        loaded.continuation,
        Some(StoredContinuation::Current(second)),
        "the retry persists the new continuation"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// `/clear` drops the continuation it understands (it described the cleared
/// conversation), keeps a newer binary's `Unsupported` record, and keeps the
/// parent lineage link.
#[test]
fn clear_drops_a_current_continuation_but_keeps_an_unsupported_one_and_the_parent() {
    let mut state = turns(5);
    let plan = plan_for(&state);
    apply_with_continuation(&mut state, &plan, SUMMARY, &digest_of).expect("applies");
    state.parent = Some(ParentLink {
        id: "parent-1".into(),
        rotation_id: "rot-1".into(),
    });
    state.apply(SlashCommand::Clear, &[]);
    assert!(
        state.continuation.is_none(),
        "/clear must drop the typed continuation: {:?}",
        state.continuation
    );
    assert_eq!(
        state.parent.as_ref().map(|link| link.id.as_str()),
        Some("parent-1"),
        "the lineage link survives /clear"
    );

    let mut state = turns(5);
    let unsupported = StoredContinuation::Unsupported(serde_json::json!({"version": 2}));
    state.continuation = Some(unsupported.clone());
    state.apply(SlashCommand::Clear, &[]);
    assert_eq!(
        state.continuation,
        Some(unsupported),
        "a newer binary's continuation must survive /clear"
    );
}
