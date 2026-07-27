use saya_store::{FsSessionStore, RedactedMessage, RedactedSession, SessionStore};

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
    };
    block_on(store.save(session)).unwrap();
    let path = root.join("session-1.json");
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(!saved.contains("secret"));
    assert!(!saved.contains("u:p@"));
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
    }))
    .unwrap();
    assert!(block_on(store.load("../good")).is_err());
    assert_eq!(block_on(store.most_recent()).unwrap().unwrap().id, "good");
    std::fs::remove_dir_all(root).unwrap();
}

fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}
