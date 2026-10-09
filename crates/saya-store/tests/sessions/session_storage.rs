use super::*;

#[test]
fn filesystem_store_round_trips_redacted_sessions_and_recovers_corruption() {
    let root = temp_root("roundtrip");
    let store = FsSessionStore::new(&root);
    let session = RedactedSession {
        id: "session-1".into(),
        profile_names: vec!["analytics".into()],
        messages: vec![RedactedMessage {
            role: "user".into(),
            content: "password=secret postgres://u:p@host/db".into(),
        }],
        turns: vec![RedactedTurn {
            user: "password=turn-secret".into(),
            assistant: "postgres://u:p@host/db".into(),
            database_derived: true,
            tools: vec![RedactedToolMetadata {
                name: "bounded_sql_query".into(),
                status: "completed".into(),
                ..Default::default()
            }],
        }],
        ..Default::default()
    };
    block_on(store.save(session)).unwrap();
    let path = root.join("session-1.json");
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(!saved.contains("secret"));
    assert!(!saved.contains("u:p@"));
    assert!(saved.contains("bounded_sql_query"));
    assert!(saved.contains("completed"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(&root).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    assert!(block_on(store.load("session-1")).unwrap().is_some());
    std::fs::write(&path, "not json").unwrap();
    assert!(block_on(store.load("session-1")).unwrap().is_none());
    assert!(
        std::fs::read_dir(&root).unwrap().any(|entry| entry
            .unwrap()
            .path()
            .to_string_lossy()
            .contains("corrupt-"))
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn filesystem_store_drops_tool_payload_metadata() {
    let root = temp_root("tool-metadata");
    let store = FsSessionStore::new(&root);
    block_on(store.save(RedactedSession {
        id: "minimal".into(),
        profile_names: vec![],
        messages: vec![],
        turns: vec![RedactedTurn {
            user: "q".into(),
            assistant: "a".into(),
            database_derived: true,
            tools: vec![RedactedToolMetadata {
                name: "bounded_sql_query".into(),
                status: "completed".into(),
                arguments: r#"{"sql":"SELECT secret FROM users"}"#.into(),
                result_shape: Some(saya_store::RedactedToolResultShape {
                    row_count: 1,
                    columns: vec!["secret".into()],
                }),
            }],
        }],
        ..Default::default()
    }))
    .unwrap();

    let saved = std::fs::read_to_string(root.join("minimal.json")).unwrap();
    assert!(saved.contains("bounded_sql_query"));
    assert!(saved.contains("completed"));
    assert!(!saved.contains("SELECT secret FROM users"));
    assert!(!saved.contains("result_shape"));
    assert!(!saved.contains("secret"));

    let loaded = block_on(store.load("minimal")).unwrap().unwrap();
    let tool = &loaded.turns[0].tools[0];
    assert!(tool.arguments.is_empty());
    assert!(tool.result_shape.is_none());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn most_recent_ignores_corrupt_sessions_and_rejects_path_traversal() {
    let root = temp_root("recent");
    let store = FsSessionStore::new(&root);
    block_on(store.save(RedactedSession {
        id: "good".into(),
        profile_names: vec![],
        messages: vec![],
        ..Default::default()
    }))
    .unwrap();
    assert!(block_on(store.load("../good")).is_err());
    assert_eq!(block_on(store.most_recent()).unwrap().unwrap().id, "good");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn repeated_saves_replace_the_same_session_on_all_platforms() {
    let root = temp_root("repeat");
    let store = FsSessionStore::new(&root);
    block_on(store.save(RedactedSession {
        id: "same".into(),
        profile_names: vec![],
        messages: vec![RedactedMessage {
            role: "user".into(),
            content: "first".into(),
        }],
        ..Default::default()
    }))
    .unwrap();
    block_on(store.save(RedactedSession {
        id: "same".into(),
        profile_names: vec![],
        messages: vec![RedactedMessage {
            role: "user".into(),
            content: "second".into(),
        }],
        ..Default::default()
    }))
    .unwrap();
    let loaded = block_on(store.load("same")).unwrap().unwrap();
    assert_eq!(loaded.messages[0].content, "second");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn redaction_handles_multiple_known_markers_and_urls() {
    let root = temp_root("redaction");
    let store = FsSessionStore::new(&root);
    block_on(store.save(RedactedSession { id: "redact".into(), profile_names: vec![], messages: vec![RedactedMessage { role: "user".into(), content: "password=one password=two api_key=aaa api_key=bbb postgres://u:p@one.test postgres://x:y@two.test".into() }], ..Default::default() })).unwrap();
    let saved = std::fs::read_to_string(root.join("redact.json")).unwrap();
    for secret in [
        "password=one",
        "password=two",
        "api_key=aaa",
        "api_key=bbb",
        "u:p@",
        "x:y@",
    ] {
        assert!(!saved.contains(secret), "leaked {secret}");
    }
    let _ = std::fs::remove_dir_all(root);
}
