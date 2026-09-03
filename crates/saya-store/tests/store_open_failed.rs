//! A store whose path can never be opened fails fast instead of retrying for
//! the full busy ceiling.
//!
//! `prepare_path` creates the database directory. When a parent path component
//! is a regular file, that creation fails permanently — the directory can never
//! exist, so the store can never open. That is a different failure from the
//! transient write-lock contention the busy ceiling exists for: a permanently
//! unopenable store must report at once, not retry for thirty seconds. The
//! busy case — a sibling process holding the write lock — is exercised by
//! `store_busy.rs` and is unchanged here; this file covers only the permanent
//! half.

use saya_store::{SchemaStore, SqliteStateStore, StoreError};
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const PROFILE: &str = "p-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-open-failed-{label}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

/// A parent that is a regular file can never become a directory, so the store
/// can never open. That permanent failure must report within a second — not
/// retry for the thirty-second busy ceiling — and must not surface as the
/// retryable `Unavailable`.
#[tokio::test]
async fn unopenable_path_fails_fast_instead_of_retrying_as_busy() {
    let root = temp_root("blocked");
    fs::write(root.join("blocker"), b"x").unwrap();
    let bad = root.join("blocker/state.sqlite3");
    let store = SqliteStateStore::new(&bad);

    let started = Instant::now();
    let open = tokio::time::timeout(Duration::from_secs(2), store.get_schema(PROFILE)).await;
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(1),
        "a permanent open failure retried for {elapsed:?} instead of failing fast"
    );
    let error = open
        .expect("a permanent open failure must resolve within a second, not time out")
        .expect_err("an unopenable store must fail, not succeed");
    assert_eq!(
        error,
        StoreError::OpenFailed,
        "a permanent path failure must surface as OpenFailed, not the retryable Unavailable"
    );

    let _ = fs::remove_dir_all(root);
}
