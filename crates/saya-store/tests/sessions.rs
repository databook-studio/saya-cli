use saya_store::{
    FsSessionStore, MAX_SESSION_BYTES, RedactedMessage, RedactedSession, RedactedToolMetadata,
    RedactedTurn, SessionHistoryQuery, SessionStore, StoreError,
};

fn temp_root(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("saya-store-{label}-{}", std::process::id()))
}

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

#[test]
fn history_lists_valid_sessions_in_recent_first_order() {
    let root = temp_root("history");
    let store = FsSessionStore::new(&root);
    block_on(store.save(RedactedSession {
        id: "older".into(),
        profile_names: vec![],
        messages: vec![],
        ..Default::default()
    }))
    .unwrap();
    let epoch = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
    std::fs::File::options()
        .write(true)
        .open(root.join("older.json"))
        .unwrap()
        .set_times(
            std::fs::FileTimes::new().set_modified(epoch + std::time::Duration::from_micros(100)),
        )
        .unwrap();
    block_on(store.save(RedactedSession {
        id: "newer".into(),
        profile_names: vec![],
        messages: vec![],
        ..Default::default()
    }))
    .unwrap();
    std::fs::File::options()
        .write(true)
        .open(root.join("newer.json"))
        .unwrap()
        .set_times(
            std::fs::FileTimes::new().set_modified(epoch + std::time::Duration::from_micros(200)),
        )
        .unwrap();
    let history = block_on(store.history(SessionHistoryQuery::first_page(2).unwrap())).unwrap();
    assert_eq!(history.entries.len(), 2);
    assert_eq!(history.entries[0].id, "newer");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn history_pages_are_bounded_and_continue_without_duplicates() {
    let root = temp_root("history_pages");
    let _ = std::fs::remove_dir_all(&root);
    let store = FsSessionStore::new(&root);
    for index in 0..7 {
        block_on(store.save(RedactedSession {
            id: format!("session-{index:02}"),
            profile_names: vec![],
            messages: vec![],
            ..Default::default()
        }))
        .unwrap();
    }

    let query = SessionHistoryQuery::first_page(3).unwrap();
    let first = block_on(store.history(query.clone())).unwrap();
    assert_eq!(first.entries.len(), 3);
    assert!(first.next_cursor.is_some());
    let second_query = query.next_page(&first).expect("first page continues");
    let second = block_on(store.history(second_query.clone())).unwrap();
    let third_query = second_query
        .next_page(&second)
        .expect("second page continues");
    let third = block_on(store.history(third_query)).unwrap();

    let ids = first
        .entries
        .into_iter()
        .chain(second.entries)
        .chain(third.entries)
        .map(|entry| entry.id)
        .collect::<Vec<_>>();
    let unique = ids.iter().collect::<std::collections::BTreeSet<_>>();
    assert_eq!(ids.len(), 7);
    assert_eq!(unique.len(), ids.len());
    assert!(third.next_cursor.is_none());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn history_page_size_is_typed_and_bounded() {
    assert!(SessionHistoryQuery::first_page(0).is_err());
    assert!(
        SessionHistoryQuery::first_page(saya_store::MAX_SESSION_HISTORY_PAGE_SIZE + 1).is_err()
    );
}

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

fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}
