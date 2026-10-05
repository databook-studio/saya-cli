//! Failure-injection tests for successor publication through the test-only
//! replacer seam: a failed atomic publish leaves the parent fully usable
//! with no successor record and no staged temps, and a retry converges to
//! exactly one child.

use crate::SessionStore;
use crate::replace::FailingReplacer;
use crate::session_lifecycle::SuccessorOutcome;
use crate::{FsSessionStore, ParentLink, RedactedMessage, RedactedSession, StoreError};
use std::path::PathBuf;

fn temp_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "saya-store-successor-{label}-{}",
        std::process::id()
    ))
}

fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}

fn message_session(id: &str, content: &str) -> RedactedSession {
    RedactedSession {
        id: id.into(),
        messages: vec![RedactedMessage {
            role: "user".into(),
            content: content.into(),
        }],
        ..Default::default()
    }
}

fn plant(store: &FsSessionStore, id: &str, content: &str) {
    block_on(store.save(message_session(id, content))).unwrap();
}

#[test]
fn a_failed_publish_leaves_the_parent_usable_and_no_successor() {
    let root = temp_root("failed-publish");
    let _ = std::fs::remove_dir_all(&root);
    let store = FsSessionStore::new(&root);
    plant(&store, "parent", "original");

    let error = store
        .publish_successor_with_replacer(
            "parent",
            "rot-1",
            message_session("child", "fresh"),
            &FailingReplacer,
        )
        .expect_err("the injected publish failure must surface");
    assert_eq!(error, StoreError::Unavailable);
    assert_eq!(
        block_on(store.load("parent")).unwrap().unwrap().messages[0].content,
        "original",
        "the parent stays fully usable"
    );
    assert!(
        !root.join("child.json").exists(),
        "no successor record is left"
    );
    assert!(
        std::fs::read_dir(&root)
            .unwrap()
            .filter_map(Result::ok)
            .all(|entry| !entry.path().extension().is_some_and(|ext| ext == "tmp")),
        "no staged temps survive a failed publish"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_retry_after_a_failed_publish_converges_to_one_child() {
    let root = temp_root("failed-retry");
    let _ = std::fs::remove_dir_all(&root);
    let store = FsSessionStore::new(&root);
    plant(&store, "parent", "original");

    assert!(
        store
            .publish_successor_with_replacer(
                "parent",
                "rot-1",
                message_session("child", "fresh"),
                &FailingReplacer,
            )
            .is_err()
    );
    assert_eq!(
        store
            .publish_successor("parent", "rot-1", message_session("child", "retry"))
            .unwrap(),
        SuccessorOutcome::Published
    );
    let child = block_on(store.load("child")).unwrap().unwrap();
    assert_eq!(
        child.parent,
        Some(ParentLink {
            id: "parent".into(),
            rotation_id: "rot-1".into()
        })
    );
    let mut names: Vec<String> = std::fs::read_dir(&root)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec!["child.json".to_owned(), "parent.json".to_owned()],
        "exactly one child record exists"
    );
    let _ = std::fs::remove_dir_all(root);
}
