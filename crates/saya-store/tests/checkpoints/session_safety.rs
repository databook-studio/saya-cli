use super::*;

#[test]
fn invalid_session_ids_are_refused_for_every_operation() {
    let (state, store) = ck_store("invalid-session");
    let after = after_of(b"v");
    let id = "ck-00000000000000000000000000000000";
    for session in ["", "..", "a/b", "a\\b"] {
        assert!(matches!(
            prepare(&store, session, "a.txt", Some(b"x"), 1_000).unwrap_err(),
            CheckpointStoreError::Store(_)
        ));
        assert!(matches!(
            store.list_checkpoints(session).unwrap_err(),
            CheckpointStoreError::Store(_)
        ));
        assert!(matches!(
            store.checkpoint_usage(session).unwrap_err(),
            CheckpointStoreError::Store(_)
        ));
        assert!(matches!(
            store.read_before_image(session, id).unwrap_err(),
            CheckpointStoreError::Store(_)
        ));
        assert!(matches!(
            store.delete_checkpoint(session, id).unwrap_err(),
            CheckpointStoreError::Store(_)
        ));
        assert!(matches!(
            store
                .commit_checkpoint(session, id, after.clone(), 1_000)
                .unwrap_err(),
            CheckpointStoreError::Store(_)
        ));
        assert!(matches!(
            store.recover_checkpoints(session).unwrap_err(),
            CheckpointStoreError::Store(_)
        ));
        assert!(
            !state.join("sessions").join(session).exists(),
            "nothing is written for an invalid session id"
        );
    }
    let _ = fs::remove_dir_all(state);
}
#[cfg(unix)]
#[test]
fn symlinked_checkpoint_paths_are_refused_and_targets_untouched() {
    use std::os::unix::fs::symlink;

    let (state, store) = ck_store("symlink");
    let victim = state.join("victim");
    fs::create_dir_all(&victim).unwrap();
    fs::write(victim.join("keep.txt"), "precious").unwrap();

    fs::create_dir_all(state.join("sessions").join("s1")).unwrap();
    symlink(
        &victim,
        state.join("sessions").join("s1").join("checkpoints"),
    )
    .unwrap();
    let error = prepare(&store, "s1", "a.txt", Some(b"x"), 1_000).unwrap_err();
    assert_eq!(error, CheckpointStoreError::Store(StoreError::Invalid));
    assert_eq!(
        fs::read(victim.join("keep.txt")).unwrap(),
        b"precious".to_vec()
    );
    assert_eq!(entry_count(&victim), 1, "the link target gained nothing");

    symlink(&victim, state.join("sessions").join("s2")).unwrap();
    let error = prepare(&store, "s2", "a.txt", Some(b"x"), 1_000).unwrap_err();
    assert_eq!(error, CheckpointStoreError::Store(StoreError::Invalid));
    assert_eq!(entry_count(&victim), 1, "the link target gained nothing");
    let _ = fs::remove_dir_all(state);
}
#[test]
fn finishing_the_deletion_removes_the_checkpoints_dir_and_leaves_siblings_intact() {
    let (state, store) = ck_store("delete");
    fs::create_dir_all(state.join("runs")).unwrap();
    fs::write(state.join("state.sqlite3"), "db").unwrap();
    fs::write(state.join("input_history"), "history").unwrap();
    fs::write(state.join("runs/step"), "run").unwrap();
    plant_session(&store, "a");
    plant_session(&store, "other");
    let kept = prepare(&store, "other", "keep.txt", Some(b"sibling"), 1_000).unwrap();
    let gone_one = prepare(&store, "a", "one.txt", Some(b"first"), 1_000).unwrap();
    let gone_two = prepare(&store, "a", "two.txt", Some(b"second"), 1_000).unwrap();

    store.begin_delete("a").unwrap();
    assert_eq!(store.finish_delete("a").unwrap(), DeleteStatus::Complete);

    assert!(!state.join("sessions").join("a").exists());
    assert!(!state.join("sessions/a.json").exists());
    assert_eq!(
        fs::read(state.join("state.sqlite3")).unwrap(),
        b"db".to_vec()
    );
    assert_eq!(
        fs::read(state.join("input_history")).unwrap(),
        b"history".to_vec()
    );
    assert_eq!(fs::read(state.join("runs/step")).unwrap(), b"run".to_vec());
    assert_eq!(block_on(store.load("other")).unwrap().unwrap().id, "other");
    let listed = store.list_checkpoints("other").unwrap();
    assert_eq!(listed, vec![kept.clone()]);
    assert_eq!(
        store.read_before_image("other", kept.id.as_str()).unwrap(),
        b"sibling".to_vec()
    );
    assert_eq!(
        store
            .read_before_image("a", gone_one.id.as_str())
            .unwrap_err(),
        CheckpointStoreError::NotFound
    );
    assert_eq!(
        store
            .read_before_image("a", gone_two.id.as_str())
            .unwrap_err(),
        CheckpointStoreError::NotFound
    );
    let _ = fs::remove_dir_all(state);
}
