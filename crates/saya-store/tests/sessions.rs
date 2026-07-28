use saya_store::{
    FsSessionStore, RedactedMessage, RedactedSession, RedactedToolMetadata, RedactedTurn,
    SessionStore,
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
    std::fs::remove_dir_all(root).unwrap();
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
    std::fs::remove_dir_all(root).unwrap();
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
    std::fs::remove_dir_all(root).unwrap();
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
    std::fs::remove_dir_all(root).unwrap();
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
    block_on(store.save(RedactedSession {
        id: "newer".into(),
        profile_names: vec![],
        messages: vec![],
        ..Default::default()
    }))
    .unwrap();
    let history = block_on(store.history()).unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].id, "newer");
    std::fs::remove_dir_all(root).unwrap();
}

fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}
