use super::*;

#[test]
fn a_legacy_session_without_a_continuation_loads_and_saves_unchanged() {
    let root = temp_root("legacy-no-continuation");
    let store = FsSessionStore::new(&root);
    std::fs::create_dir_all(&root).unwrap();
    let legacy = serde_json::json!({
        "version": 2,
        "id": "legacy",
        "provider": "postgres",
        "model": "demo",
        "approval_mode": "confirm",
        "turns": [{"user": "u", "assistant": "a"}],
        "messages": []
    });
    std::fs::write(
        root.join("legacy.json"),
        serde_json::to_string(&legacy).unwrap(),
    )
    .unwrap();

    let loaded = block_on(store.load("legacy")).unwrap().unwrap();
    assert!(loaded.continuation.is_none(), "absence stays absence");

    block_on(store.save(loaded)).unwrap();
    let saved = std::fs::read_to_string(root.join("legacy.json")).unwrap();
    assert!(
        !saved.contains("\"continuation\""),
        "a legacy save must not grow the continuation key: {saved}"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_safe_continuation_round_trips() {
    let root = temp_root("continuation-roundtrip");
    let store = FsSessionStore::new(&root);
    let record = safe_continuation(
        "wrapped up the migration",
        "double-check the index",
        "cargo test",
    );

    block_on(store.save(RedactedSession {
        id: "continued".into(),
        continuation: Some(StoredContinuation::Current(record.clone())),
        ..Default::default()
    }))
    .unwrap();

    let saved = std::fs::read_to_string(root.join("continued.json")).unwrap();
    assert!(saved.contains("\"continuation\""));
    assert!(saved.contains("\"through_turn\""));

    let loaded = block_on(store.load("continued")).unwrap().unwrap();
    let Some(StoredContinuation::Current(round_tripped)) = loaded.continuation else {
        panic!(
            "a typed continuation must load back typed: {:?}",
            loaded.continuation
        );
    };
    assert_eq!(round_tripped, record);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn an_unknown_version_continuation_is_preserved_on_load() {
    let root = temp_root("continuation-unknown-version");
    let store = FsSessionStore::new(&root);
    std::fs::create_dir_all(&root).unwrap();
    let future = serde_json::json!({"version": 2, "future": true});
    let file = serde_json::json!({
        "version": 2,
        "id": "future",
        "continuation": future
    });
    std::fs::write(
        root.join("future.json"),
        serde_json::to_string(&file).unwrap(),
    )
    .unwrap();

    let loaded = block_on(store.load("future")).unwrap().unwrap();
    let Some(StoredContinuation::Unsupported(value)) = &loaded.continuation else {
        panic!(
            "an unknown version must load as Unsupported: {:?}",
            loaded.continuation
        );
    };
    assert_eq!(value, &future);

    block_on(store.save(loaded)).unwrap();
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("future.json")).unwrap()).unwrap();
    assert_eq!(saved["continuation"], future);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_save_without_the_extension_cannot_overwrite_an_unsupported_one() {
    let root = temp_root("continuation-guard");
    let store = FsSessionStore::new(&root);
    std::fs::create_dir_all(&root).unwrap();
    let file = serde_json::json!({
        "version": 2,
        "id": "future",
        "continuation": {"version": 2, "future": true}
    });
    std::fs::write(
        root.join("future.json"),
        serde_json::to_string(&file).unwrap(),
    )
    .unwrap();
    let before = std::fs::read(root.join("future.json")).unwrap();

    let error = block_on(store.save(RedactedSession {
        id: "future".into(),
        ..Default::default()
    }))
    .expect_err("a save that would drop an unsupported continuation must be refused");
    assert_eq!(error, StoreError::UnsupportedContinuation);
    assert_eq!(
        std::fs::read(root.join("future.json")).unwrap(),
        before,
        "the guard refusal must leave the record byte-for-byte intact"
    );
    let _ = std::fs::remove_dir_all(root);
}
