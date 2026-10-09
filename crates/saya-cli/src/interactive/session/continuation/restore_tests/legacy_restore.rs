/// A session written before continuations existed resumes byte-for-byte as
/// before: the raw fixture carries no `continuation` key, and
/// deserialization — not struct construction — proves the old record loads.
#[test]
fn an_old_session_without_a_continuation_field_resumes_unchanged() {
    let root = store_root("legacy");
    std::fs::write(
        root.join("old.json"),
        r#"{"version":2,"id":"old","profile_names":[],"turns":[{"user":"q0","assistant":"a0"},{"user":"q1","assistant":"a1"},{"user":"q2","assistant":"a2"},{"user":"q3","assistant":"a3"},{"user":"q4","assistant":"a4"},{"user":"q5","assistant":"a5"}],"messages":[]}"#,
    )
    .expect("writes the legacy fixture");
    let store = FsSessionStore::new(&root);
    let loaded = block_on(store.load("old"))
        .expect("loads")
        .expect("present");
    let mut resumed = state_from_redacted(loaded, &defaults());
    assert!(matches!(
        restore_continuation(&mut resumed),
        RestoreOutcome::NoContinuation
    ));
    assert_resumed_unchanged(&resumed, 6);
    assert_eq!(
        resumed.provider_request_history()[0].content,
        "q0",
        "the old record replays from its first turn, byte-for-byte"
    );
    let _ = std::fs::remove_dir_all(root);
}
