use super::*;

#[test]
fn begin_delete_writes_a_tombstone_outside_the_sessions_tree_and_hides_the_session() {
    let (state, store) = deletion_store("tombstone-hide");
    plant_state_sentinels(&state);
    plant_session(&store, "other", "keep");
    plant_session(&store, "a", "hello");
    let session_path = state.join("sessions/a.json");
    let before = std::fs::read(&session_path).unwrap();

    store.begin_delete("a").unwrap();

    let tombstone = state.join("lifecycle/session-a.tombstone");
    assert!(
        tombstone.is_file(),
        "the tombstone lives under <state>/lifecycle/"
    );
    let sessions = std::fs::read_dir(state.join("sessions")).unwrap();
    assert!(
        sessions.filter_map(Result::ok).all(|entry| entry
            .path()
            .extension()
            .is_none_or(|ext| ext != "tombstone")),
        "no tombstone is named inside <sessions>"
    );

    assert!(store.is_deleted("a").unwrap());
    assert!(
        block_on(store.load("a")).unwrap().is_none(),
        "load hides a tombstoned session at once"
    );
    assert_eq!(history_ids(&store, 10), vec!["other".to_owned()]);
    let recent = block_on(store.most_recent())
        .unwrap()
        .expect("the surviving session is still listed");
    assert_eq!(
        recent.id, "other",
        "most_recent skips the tombstoned session"
    );

    let error = block_on(store.save(message_session("a", "resurrect")))
        .expect_err("a save of a tombstoned session is refused");
    assert_eq!(error, StoreError::Conflict);
    assert_eq!(
        std::fs::read(&session_path).unwrap(),
        before,
        "the previous bytes stay untouched until finish"
    );
    let _ = std::fs::remove_dir_all(state);
}
#[test]
fn finish_delete_removes_the_session_its_state_dir_staged_temps_and_quarantine_and_the_tombstone() {
    let (state, store) = deletion_store("finish-complete");
    plant_state_sentinels(&state);
    plant_session(&store, "a", "hello");
    plant_session(&store, "other", "keep");
    plant_session(&store, "a-2", "keep too");
    let sessions = state.join("sessions");
    std::fs::create_dir_all(sessions.join("a/run_program")).unwrap();
    std::fs::create_dir_all(sessions.join("other")).unwrap();
    std::fs::write(sessions.join("a/journal.ndjson"), "j").unwrap();
    std::fs::write(sessions.join("a/run_program/x.json"), "{}").unwrap();
    std::fs::write(sessions.join("a/scratch.duckdb"), "db").unwrap();
    std::fs::write(sessions.join("other/journal.ndjson"), "j").unwrap();
    std::fs::write(sessions.join(".a.json.12.3.tmp"), "staged").unwrap();
    std::fs::write(sessions.join("a.corrupt-111"), "quarantine").unwrap();
    std::fs::write(sessions.join(".a-2.json.1.1.tmp"), "keep").unwrap();
    std::fs::write(sessions.join("a.corrupt-x"), "keep").unwrap();

    store.begin_delete("a").unwrap();
    assert_eq!(store.pending_deletions().unwrap(), vec!["a".to_owned()]);
    let status = store.finish_delete("a").unwrap();

    assert_eq!(status, DeleteStatus::Complete);
    assert!(!sessions.join("a.json").exists());
    assert!(!sessions.join("a").exists());
    assert!(!sessions.join(".a.json.12.3.tmp").exists());
    assert!(!sessions.join("a.corrupt-111").exists());
    assert!(!state.join("lifecycle/session-a.tombstone").exists());
    assert_eq!(store.pending_deletions().unwrap(), Vec::<String>::new());
    // Independent data — the state dir, other sessions, look-alike names.
    assert_eq!(
        std::fs::read(state.join("state.sqlite3")).unwrap(),
        "db".as_bytes().to_vec()
    );
    assert_eq!(
        std::fs::read(state.join("input_history")).unwrap(),
        "history".as_bytes().to_vec()
    );
    assert_eq!(
        std::fs::read(state.join("runs/step")).unwrap(),
        "run".as_bytes().to_vec()
    );
    assert!(sessions.join("other.json").is_file());
    assert!(sessions.join("other/journal.ndjson").is_file());
    assert!(sessions.join("a-2.json").is_file());
    assert_eq!(
        std::fs::read(sessions.join(".a-2.json.1.1.tmp")).unwrap(),
        "keep".as_bytes().to_vec()
    );
    assert_eq!(
        std::fs::read(sessions.join("a.corrupt-x")).unwrap(),
        "keep".as_bytes().to_vec()
    );
    assert_eq!(block_on(store.load("other")).unwrap().unwrap().id, "other");
    let _ = std::fs::remove_dir_all(state);
}
#[cfg(unix)]
#[test]
fn deleting_never_follows_a_symlink() {
    use std::os::unix::fs::symlink;

    let (state, store) = deletion_store("no-follow");
    plant_state_sentinels(&state);
    let sessions = state.join("sessions");
    let victim = state.join("victim.txt");
    std::fs::write(&victim, "precious").unwrap();

    // A link planted INSIDE the id dir is unlinked as a name; its target
    // keeps its bytes.
    std::fs::create_dir_all(sessions.join("a")).unwrap();
    std::fs::write(sessions.join("a/real.ndjson"), "real").unwrap();
    symlink(&victim, sessions.join("a/link")).unwrap();
    store.begin_delete("a").unwrap();
    assert_eq!(store.finish_delete("a").unwrap(), DeleteStatus::Complete);
    assert!(
        !sessions.join("a").exists(),
        "the id dir including the link is gone"
    );
    assert_eq!(
        std::fs::read(&victim).unwrap(),
        "precious".as_bytes().to_vec(),
        "the link target keeps its bytes"
    );

    // The id dir itself replaced by a symlink: only the link is removed,
    // the outside directory and its contents are intact.
    let outside = state.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("keep.txt"), "keep").unwrap();
    symlink(&outside, sessions.join("b")).unwrap();
    store.begin_delete("b").unwrap();
    assert_eq!(store.finish_delete("b").unwrap(), DeleteStatus::Complete);
    assert!(!sessions.join("b").exists(), "only the link is removed");
    assert!(outside.is_dir());
    assert_eq!(
        std::fs::read(outside.join("keep.txt")).unwrap(),
        "keep".as_bytes().to_vec()
    );
    let _ = std::fs::remove_dir_all(state);
}
#[test]
fn traversal_ids_are_refused() {
    let (state, store) = deletion_store("traversal");
    plant_state_sentinels(&state);
    plant_session(&store, "safe", "here");
    let sentinel = std::fs::read(state.join("state.sqlite3")).unwrap();

    for id in ["../x", "a/b", "a\\b", "", ".", ".."] {
        assert_eq!(
            store.begin_delete(id).unwrap_err(),
            StoreError::Unavailable,
            "begin_delete refuses {id:?}"
        );
        assert_eq!(
            store.finish_delete(id).unwrap_err(),
            StoreError::Unavailable,
            "finish_delete refuses {id:?}"
        );
    }

    assert!(
        !state.join("lifecycle").exists(),
        "no tombstone tree is created for refused ids"
    );
    assert_eq!(store.pending_deletions().unwrap(), Vec::<String>::new());
    assert_eq!(
        std::fs::read(state.join("state.sqlite3")).unwrap(),
        sentinel
    );
    assert!(state.join("sessions/safe.json").is_file());
    let _ = std::fs::remove_dir_all(state);
}
