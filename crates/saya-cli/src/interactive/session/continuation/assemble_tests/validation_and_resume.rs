/// An observation set that cannot validate refuses the whole assembly, and
/// the previous continuation stays untouched.
///
/// Deterministic shape, documented: the builder maps observations to facts
/// without re-filtering paths (the collector enforces the contract's path
/// rule at absorb time), so a fact whose path escapes the workspace — here a
/// hand-carried observation, the only way past a collector — fails
/// `validate()`, and `assemble_continuation` returns `None`.
#[test]
fn a_failed_validation_keeps_the_previous_continuation() {
    let mut state = turns(5);
    state.continuation = Some(StoredContinuation::Current(seeded_current(Some(
        "previous",
    ))));
    state.observations.files.push(FileObservation {
        path: "../escape.rs".into(),
        tool: "workspace_edit".into(),
        observed_unix_ms: NOW,
    });
    let plan = plan_for(&state);
    apply_with_continuation(&mut state, &plan, SUMMARY, &digest_of).expect("applies");
    let record = current(&state);
    assert_eq!(
        record.user_notes.as_deref(),
        Some("previous"),
        "the previous continuation is untouched"
    );
    assert_eq!(
        record.source.source_sha256,
        "b".repeat(64),
        "no new record replaced the seeded one"
    );
    assert!(
        record.files.is_empty(),
        "the invalid fact was not persisted"
    );
}

/// A typed continuation round-trips: `redacted()` hands it to the store, the
/// store persists it, and a resume carries the same record back.
#[test]
fn a_continuation_round_trips_through_redacted_and_resume() {
    let root = store_root("round-trip");
    let store = FsSessionStore::new(&root);
    let mut state = turns(2);
    observe_edit(&mut state, "src/a.rs");
    let record = assemble_continuation(&state, "carried summary.", &digest_of).expect("assembles");
    state.continuation = Some(StoredContinuation::Current(record));
    block_on(store.save(state.redacted())).expect("the record saves");
    let loaded = block_on(store.load("s1"))
        .expect("loads")
        .expect("the record is there");
    let resumed = state_from_redacted(loaded, &defaults());
    assert_eq!(
        resumed.continuation, state.continuation,
        "the typed continuation round-trips through the store and resume"
    );
    assert_eq!(resumed.parent, state.parent);
    let _ = std::fs::remove_dir_all(root);
}

/// A continuation this binary does not understand loads as `Unsupported`,
/// survives a resume, and a save carrying it verbatim succeeds while a save
/// that would drop it is refused — the old `redacted()` (hard-coding
/// `continuation: None`) would have made every later save of the session
/// fail.
#[test]
fn an_unsupported_continuation_survives_resume_and_save() {
    let root = store_root("unsupported");
    let store = FsSessionStore::new(&root);
    let state = turns(2);
    block_on(store.save(state.redacted())).expect("the base record saves");
    let path = root.join("s1.json");
    let mut value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("parses");
    value["continuation"] = serde_json::json!({"version": 2, "future": true});
    std::fs::write(&path, serde_json::to_string_pretty(&value).expect("writes")).expect("write");

    let loaded = block_on(store.load("s1")).expect("loads").expect("present");
    let resumed = state_from_redacted(loaded, &defaults());
    let Some(StoredContinuation::Unsupported(raw)) = &resumed.continuation else {
        panic!(
            "an unknown continuation must load as Unsupported: {:?}",
            resumed.continuation
        );
    };
    assert_eq!(raw, &serde_json::json!({"version": 2, "future": true}));

    block_on(store.save(resumed.redacted()))
        .expect("a save carrying the Unsupported record verbatim succeeds");
    let file_value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("parses");
    assert_eq!(
        file_value["continuation"],
        serde_json::json!({"version": 2, "future": true}),
        "the unsupported record is preserved verbatim: {file_value}"
    );

    let mut dropped = resumed.redacted();
    dropped.continuation = None;
    let error = block_on(store.save(dropped))
        .expect_err("dropping an unsupported continuation must be refused");
    assert_eq!(error, StoreError::UnsupportedContinuation);
    let _ = std::fs::remove_dir_all(root);
}
