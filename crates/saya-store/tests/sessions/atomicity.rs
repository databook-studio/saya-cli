use super::*;

#[test]
fn oversized_session_is_rejected_without_replacing_existing_record() {
    let root = temp_root("session_bound");
    let store = FsSessionStore::new(&root);
    block_on(store.save(RedactedSession {
        id: "bounded".into(),
        profile_names: vec![],
        messages: vec![RedactedMessage {
            role: "user".into(),
            content: "old".into(),
        }],
        ..Default::default()
    }))
    .unwrap();

    let error = block_on(store.save(RedactedSession {
        id: "bounded".into(),
        profile_names: vec![],
        messages: vec![RedactedMessage {
            role: "user".into(),
            content: "x".repeat(MAX_SESSION_BYTES),
        }],
        ..Default::default()
    }))
    .expect_err("an oversized session must be refused");
    assert_eq!(error, StoreError::LimitExceeded);
    assert_eq!(
        block_on(store.load("bounded")).unwrap().unwrap().messages[0].content,
        "old",
        "a refused save cannot replace the prior complete record"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn concurrent_saves_use_independent_atomic_temps() {
    let root = temp_root("concurrent");
    let store = FsSessionStore::new(&root);
    let first = RedactedSession {
        id: "same".into(),
        profile_names: vec![],
        messages: vec![RedactedMessage {
            role: "user".into(),
            content: "first".repeat(100_000),
        }],
        ..Default::default()
    };
    let second = RedactedSession {
        id: "same".into(),
        profile_names: vec![],
        messages: vec![RedactedMessage {
            role: "user".into(),
            content: "second".repeat(100_000),
        }],
        ..Default::default()
    };
    let (left, right) = block_on(async { tokio::join!(store.save(first), store.save(second)) });
    assert!(left.is_ok(), "first concurrent save failed: {left:?}");
    assert!(right.is_ok(), "second concurrent save failed: {right:?}");
    let loaded = block_on(store.load("same")).unwrap().unwrap();
    assert!(
        loaded.messages[0].content.starts_with("first")
            || loaded.messages[0].content.starts_with("second"),
        "the winner must be one complete save"
    );
    let _ = std::fs::remove_dir_all(root);
}
