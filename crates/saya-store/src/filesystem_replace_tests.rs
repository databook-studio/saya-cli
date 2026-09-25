//! Regression tests for non-atomic session replacement (A012): a failed
//! publish must preserve the last good session, on every platform. The
//! tests inject a failing [`Replacer`](super::super::replace::Replacer)
//! through the [`publish_staged`](super::super::replace::publish_staged)
//! seam, so no Windows host is needed.

use super::super::replace::{FailingReplacer, Replacer};
use crate::{FsSessionStore, MAX_SESSION_BYTES, RedactedSession, SessionStore, StoreError};
use std::path::PathBuf;

fn temp_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!("saya-store-{label}-{}", std::process::id()))
}

fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}

fn session_with_content(id: &str, content: &str) -> RedactedSession {
    RedactedSession {
        id: id.into(),
        messages: vec![crate::RedactedMessage {
            role: "user".into(),
            content: content.into(),
        }],
        ..Default::default()
    }
}

/// Documents the pre-fix Windows failure: a copy-then-remove publish that
/// truncates the target and then fails leaves 8 unparseable bytes where the
/// last good session was. The committed regression test below injects
/// [`FailingReplacer`] instead — the contract the real atomic publish
/// satisfies — and asserts the last good record survives.
#[test]
fn copy_then_remove_failure_mode_truncates_the_target() {
    struct PartialCopy;

    impl Replacer for PartialCopy {
        fn replace(
            &self,
            temp: &std::path::Path,
            target: &std::path::Path,
        ) -> Result<(), StoreError> {
            std::fs::write(target, &b"truncated-new"[..8]).unwrap();
            let _ = std::fs::remove_file(temp);
            Err(StoreError::unavailable())
        }
    }

    let root = temp_root("partial-copy-demo");
    let _ = std::fs::remove_dir_all(&root);
    let store = FsSessionStore::new(&root);
    block_on(store.save(session_with_content("s", "old"))).unwrap();

    let result = block_on(store.save_with_replacer(session_with_content("s", "new"), &PartialCopy));
    assert!(result.is_err());
    let raw = std::fs::read(root.join("s.json")).expect("target still exists");
    assert_eq!(
        raw,
        b"truncate".to_vec(),
        "the old gap truncates the target before failing: last good data lost"
    );
    assert!(
        serde_json::from_slice::<RedactedSession>(&raw).is_err(),
        "the truncated target no longer parses"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn failed_publish_keeps_the_last_good_session() {
    let root = temp_root("publish-failure");
    let _ = std::fs::remove_dir_all(&root);
    let store = FsSessionStore::new(&root);
    block_on(store.save(session_with_content("s", "old"))).unwrap();

    let result =
        block_on(store.save_with_replacer(session_with_content("s", "new"), &FailingReplacer));
    assert!(result.is_err(), "the injected publish failure must surface");
    // RED (pre-fix) demonstration: `PartialCopy` above truncates the target
    // to 8 bytes, so the last good session is unrecoverable through this
    // seam. GREEN (post-fix): `save` routes through `publish_staged`, and
    // the real AtomicReplace is a single rename that never partially
    // overwrites, so the old record survives every publish failure.
    let raw = std::fs::read(root.join("s.json")).expect("target still exists");
    let loaded: RedactedSession = serde_json::from_slice(&raw).expect("target still parses");
    assert_eq!(
        loaded.messages[0].content, "old",
        "a failed publish must preserve the last good session"
    );
    // No staged temp may survive a failed publish.
    assert!(
        std::fs::read_dir(&root)
            .unwrap()
            .filter_map(Result::ok)
            .all(|entry| !entry.path().extension().is_some_and(|ext| ext == "tmp")),
        "a failed publish must not leave staged temps behind"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn oversized_save_rejects_before_touching_the_last_good_record() {
    let root = temp_root("publish-bound");
    let _ = std::fs::remove_dir_all(&root);
    let store = FsSessionStore::new(&root);
    block_on(store.save(session_with_content("s", "old"))).unwrap();
    let error = block_on(store.save(session_with_content("s", &"x".repeat(MAX_SESSION_BYTES))))
        .unwrap_err();
    assert_eq!(error, StoreError::LimitExceeded);
    assert_eq!(
        block_on(store.load("s")).unwrap().unwrap().messages[0].content,
        "old"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// Serialization refuses at the ceiling instead of after a full
/// materialization: a value whose pretty JSON exceeds the bound errors out
/// of the bounded writer, with the retained prefix capped at the bound.
#[test]
fn oversized_serialization_fails_fast_inside_the_bound() {
    use crate::bounded::BoundedWriter;
    let oversized = session_with_content("s", &"x".repeat(MAX_SESSION_BYTES + 1));
    let mut capped = BoundedWriter::new(Vec::new(), MAX_SESSION_BYTES);
    let result = serde_json::to_writer_pretty(&mut capped, &oversized);
    let error = result.expect_err("an oversized session must refuse mid-stream");
    assert_eq!(
        error.io_error_kind(),
        Some(std::io::ErrorKind::QuotaExceeded),
        "the refusal is the writer's bound, not a serialization failure"
    );
    assert!(
        capped.peak() <= MAX_SESSION_BYTES,
        "peak retained bytes stay within the bound: {}",
        capped.peak()
    );
}
