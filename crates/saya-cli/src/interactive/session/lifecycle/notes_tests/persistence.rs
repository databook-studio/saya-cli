use super::*;

/// A stored note round-trips through a real `FsSessionStore`: the session
/// is saved with the notes on the record, and a resume reads them back.
#[test]
fn the_notes_round_trip_through_a_real_store() {
    let root = std::env::temp_dir().join(format!(
        "saya-notes-roundtrip-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("create the store root");
    let store = FsSessionStore::new(&root);
    let mut state = state_with_current(None);
    let (ok, message) = run_notes(&mut state, NotesCommand::Set("persist these".into()));
    assert!(ok, "{message}");
    block_on(store.save(state.redacted())).expect("the record saves");
    let loaded = block_on(store.load("s1"))
        .expect("loads")
        .expect("the record is there");
    let resumed = state_from_redacted(
        loaded,
        &SessionDefaults {
            provider: "ollama".into(),
            model: "m".into(),
            allow_data_sharing: false,
            approval_mode: "ask".into(),
        },
    );
    assert_eq!(
        current(&resumed).user_notes.as_deref(),
        Some("persist these"),
        "the notes ride the store and the resume"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// A compaction after `set` keeps the notes: they ride the new record,
/// preserved apart from the generated narrative.
#[test]
fn a_compaction_after_set_keeps_the_notes() {
    let mut state = state_with_current(None);
    for index in 0..5 {
        state.record_turn(format!("q{index}"), format!("a{index}"), false, Vec::new());
    }
    let (ok, message) = run_notes(&mut state, NotesCommand::Set("keep these".into()));
    assert!(ok, "{message}");
    let plan = compact_plan(&state.turns, &state.provider_history()).expect("a long history plans");
    apply_with_continuation(&mut state, &plan, "older turns discussed.", &|_| None)
        .expect("applies");
    assert_eq!(
        current(&state).user_notes.as_deref(),
        Some("keep these"),
        "the notes ride the new record across the compaction"
    );
    assert_eq!(
        current(&state).source.through_turn,
        5,
        "the boundary is the new compaction's"
    );
}
