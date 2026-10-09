//! Session-lifecycle coordination tests: tombstone-checked lock
//! acquisition and lock-coordinated deletion over the store's two-phase
//! tombstone mechanics.
//!
//! The family rig — the scratch root, the seeded session, and the
//! deleted-session assertion — stays here; the guarantees live beside it:
//! `acquisition` (tombstone-checked lock acquisition), `deletion`
//! (lock-coordinated deletion), `exclusivity` (acquisition and deletion
//! never both win), and `validation` (invalid ids refused everywhere).

use std::path::PathBuf;

use saya_harness::HarnessError;
use saya_store::{FsSessionStore, RedactedSession, SessionStore};

#[path = "session_lifecycle/acquisition.rs"]
mod acquisition;

#[path = "session_lifecycle/deletion.rs"]
mod deletion;

#[path = "session_lifecycle/exclusivity.rs"]
mod exclusivity;

#[path = "session_lifecycle/validation.rs"]
mod validation;

fn temp_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "saya-harness-session-lifecycle-{label}-{}",
        std::process::id()
    ))
}

async fn save_session(store: &FsSessionStore, id: &str) {
    store
        .save(RedactedSession {
            id: id.to_owned(),
            ..RedactedSession::default()
        })
        .await
        .unwrap();
}

fn assert_session_deleted(error: HarnessError) {
    match error {
        HarnessError::SessionDeleted { .. } => {}
        other => panic!("expected SessionDeleted, got {other:?}"),
    }
}
