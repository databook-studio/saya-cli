use super::*;

#[test]
fn invalid_ids_and_self_rotation_are_refused() {
    let root = temp_root("successor-invalid");
    let _ = std::fs::remove_dir_all(&root);
    let store = FsSessionStore::new(&root);
    plant_session(&store, "safe", "here");
    let long_rotation = "r".repeat(65);
    let cases: Vec<(&str, &str, &str)> = vec![
        ("../x", "ok", "rot-1"),
        ("safe", "a/b", "rot-1"),
        ("", "ok", "rot-1"),
        ("safe", "", "rot-1"),
        ("safe", "safe", "rot-1"),
        ("safe", "ok", ""),
        ("safe", "ok", "rot/1"),
        ("safe", "ok", "rot.1"),
        ("safe", "ok", "r ot"),
        ("safe", "ok", long_rotation.as_str()),
        ("safe", "ok", "rôt"),
    ];
    for (parent, child, rotation) in &cases {
        let error = store
            .publish_successor(parent, rotation, message_session(child, "x"))
            .unwrap_err();
        assert_eq!(
            error,
            StoreError::Invalid,
            "refuses {parent:?}/{child:?}/{rotation:?}"
        );
    }

    let entries: Vec<String> = std::fs::read_dir(&root)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        entries,
        vec!["safe.json".to_owned()],
        "refused rotations create nothing"
    );

    // The 64-character boundary itself is a valid rotation id.
    let long_ok = "r".repeat(64);
    assert_eq!(
        store
            .publish_successor("safe", &long_ok, message_session("child64", "x"))
            .unwrap(),
        SuccessorOutcome::Published
    );
    assert_eq!(
        block_on(store.load("child64")).unwrap().unwrap().parent,
        Some(successor_link("safe", &long_ok))
    );
    let _ = std::fs::remove_dir_all(root);
}
#[test]
fn a_seed_naming_a_foreign_parent_is_invalid() {
    let root = temp_root("successor-foreign");
    let _ = std::fs::remove_dir_all(&root);
    let store = FsSessionStore::new(&root);
    plant_session(&store, "parent", "original");

    let mut seed = message_session("child", "fresh");
    seed.parent = Some(successor_link("elsewhere", "rot-1"));
    let error = store
        .publish_successor("parent", "rot-1", seed)
        .unwrap_err();
    assert_eq!(error, StoreError::Invalid, "a foreign seed link is refused");
    assert!(
        !root.join("child.json").exists(),
        "nothing is written for a foreign seed"
    );

    // A seed whose own link matches the requested rotation publishes; the
    // store sets the link itself either way.
    let mut seed = message_session("child", "fresh");
    seed.parent = Some(successor_link("parent", "rot-1"));
    assert_eq!(
        store.publish_successor("parent", "rot-1", seed).unwrap(),
        SuccessorOutcome::Published
    );
    assert_eq!(
        block_on(store.load("child")).unwrap().unwrap().parent,
        Some(successor_link("parent", "rot-1"))
    );
    let _ = std::fs::remove_dir_all(root);
}
