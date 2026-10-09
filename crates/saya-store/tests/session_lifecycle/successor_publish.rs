use super::*;

#[test]
fn a_successor_links_to_its_parent_and_leaves_the_parent_byte_identical() {
    let root = temp_root("successor-link");
    let _ = std::fs::remove_dir_all(&root);
    let store = FsSessionStore::new(&root);
    plant_session(&store, "parent", "original");
    let parent_path = root.join("parent.json");
    let before = std::fs::read(&parent_path).unwrap();

    let outcome = store
        .publish_successor("parent", "rot-1", message_session("child", "fresh"))
        .unwrap();

    assert_eq!(outcome, SuccessorOutcome::Published);
    assert_eq!(
        std::fs::read(&parent_path).unwrap(),
        before,
        "rotation never rewrites the parent record"
    );
    let child = block_on(store.load("child")).unwrap().unwrap();
    assert_eq!(child.parent, Some(successor_link("parent", "rot-1")));
    assert_eq!(
        child.messages[0].content, "fresh",
        "the successor carries only its own seed content, not the parent's"
    );
    assert_eq!(
        block_on(store.load("parent")).unwrap().unwrap().messages[0].content,
        "original",
        "the parent stays fully usable"
    );
    let _ = std::fs::remove_dir_all(root);
}
#[test]
fn repeating_the_same_rotation_yields_one_child() {
    let root = temp_root("successor-repeat");
    let _ = std::fs::remove_dir_all(&root);
    let store = FsSessionStore::new(&root);
    plant_session(&store, "parent", "original");

    assert_eq!(
        store
            .publish_successor("parent", "rot-1", message_session("child", "first"))
            .unwrap(),
        SuccessorOutcome::Published
    );
    let child_path = root.join("child.json");
    let before = std::fs::read(&child_path).unwrap();

    // The seed is ignored on a repeat — even a materially different one.
    let second = message_session("child", "second seed that differs");
    assert_eq!(
        store.publish_successor("parent", "rot-1", second).unwrap(),
        SuccessorOutcome::AlreadyPublished
    );
    assert_eq!(
        std::fs::read(&child_path).unwrap(),
        before,
        "the stored child stays byte-identical across the retry"
    );
    let mut ids = history_ids(&store, 10);
    ids.sort();
    assert_eq!(
        ids,
        vec!["child".to_owned(), "parent".to_owned()],
        "exactly the parent plus one child"
    );
    let _ = std::fs::remove_dir_all(root);
}
#[test]
fn a_different_rotation_over_an_existing_successor_id_conflicts() {
    let root = temp_root("successor-different-rot");
    let _ = std::fs::remove_dir_all(&root);
    let store = FsSessionStore::new(&root);
    plant_session(&store, "parent", "original");
    store
        .publish_successor("parent", "rot-1", message_session("child", "first"))
        .unwrap();
    let child_path = root.join("child.json");
    let before = std::fs::read(&child_path).unwrap();

    let error = store
        .publish_successor("parent", "rot-2", message_session("child", "again"))
        .unwrap_err();

    assert_eq!(error, StoreError::Conflict);
    assert_eq!(
        std::fs::read(&child_path).unwrap(),
        before,
        "the existing child is untouched"
    );
    assert_eq!(
        block_on(store.load("child")).unwrap().unwrap().parent,
        Some(successor_link("parent", "rot-1")),
        "the original rotation link survives"
    );
    let _ = std::fs::remove_dir_all(root);
}
#[test]
fn a_plain_session_at_the_successor_id_conflicts() {
    let root = temp_root("successor-plain");
    let _ = std::fs::remove_dir_all(&root);
    let store = FsSessionStore::new(&root);
    plant_session(&store, "parent", "original");
    plant_session(&store, "child", "plain");
    let child_path = root.join("child.json");
    let before = std::fs::read(&child_path).unwrap();

    let error = store
        .publish_successor(
            "parent",
            "rot-1",
            message_session("child", "would overwrite"),
        )
        .unwrap_err();

    assert_eq!(error, StoreError::Conflict);
    assert_eq!(
        std::fs::read(&child_path).unwrap(),
        before,
        "a plain session is never replaced by a rotation"
    );
    let _ = std::fs::remove_dir_all(root);
}
#[test]
fn an_oversize_seed_is_refused_and_both_records_survive() {
    let root = temp_root("successor-oversize");
    let _ = std::fs::remove_dir_all(&root);
    let store = FsSessionStore::new(&root);
    plant_session(&store, "parent", "original");
    let parent_path = root.join("parent.json");
    let before = std::fs::read(&parent_path).unwrap();

    let error = store
        .publish_successor(
            "parent",
            "rot-1",
            session_emitting("child", MAX_SESSION_BYTES + 1),
        )
        .unwrap_err();

    assert_eq!(error, StoreError::LimitExceeded);
    assert_eq!(
        std::fs::read(&parent_path).unwrap(),
        before,
        "the parent record is untouched"
    );
    assert!(!root.join("child.json").exists(), "no successor is written");
    let _ = std::fs::remove_dir_all(root);
}
#[test]
fn a_missing_or_corrupt_parent_is_refused() {
    let root = temp_root("successor-parent-missing");
    let _ = std::fs::remove_dir_all(&root);
    let store = FsSessionStore::new(&root);

    let error = store
        .publish_successor("ghost", "rot-1", message_session("child", "x"))
        .unwrap_err();
    assert_eq!(error, StoreError::NotFound, "a missing parent is refused");
    assert!(!root.join("child.json").exists());
    assert!(!root.exists(), "nothing is created for a refused rotation");

    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("broken.json"), "not json at all").unwrap();
    let error = store
        .publish_successor("broken", "rot-1", message_session("child2", "x"))
        .unwrap_err();
    assert_eq!(
        error,
        StoreError::NotFound,
        "a corrupt parent is not usable (existing load quarantine)"
    );
    assert!(
        !root.join("child2.json").exists(),
        "no successor is written"
    );
    let _ = std::fs::remove_dir_all(root);
}
#[test]
fn tombstoned_parent_or_successor_is_refused() {
    let (state, store) = deletion_store("successor-tombstones");
    plant_session(&store, "parent", "original");

    // A tombstoned successor id is refused even without a record on disk.
    store.begin_delete("gone").unwrap();
    let error = store
        .publish_successor("parent", "rot-1", message_session("gone", "x"))
        .unwrap_err();
    assert_eq!(error, StoreError::Conflict);
    assert!(!state.join("sessions/gone.json").exists());

    // A tombstoned parent is refused; nothing is written for the child.
    store.begin_delete("parent").unwrap();
    let error = store
        .publish_successor("parent", "rot-1", message_session("child", "x"))
        .unwrap_err();
    assert_eq!(error, StoreError::Conflict);
    assert!(!state.join("sessions/child.json").exists());
    let _ = std::fs::remove_dir_all(state);
}
