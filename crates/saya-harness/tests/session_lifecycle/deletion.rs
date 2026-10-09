//! Lock-coordinated deletion: a live holder in another process refuses
//! deletion and changes nothing, and a completed deletion removes the
//! session record, the per-session state dir with everything in it, and
//! the tombstone, reports `Complete`, and leaves independent state beside
//! the sessions tree untouched.

use super::{save_session, temp_root};
use saya_harness::HarnessError;
use saya_harness::session_lifecycle::delete_session;
use saya_store::{DeleteStatus, FsSessionStore, SessionStore};

/// c. A live holder in another process refuses deletion and changes
/// nothing: the record still loads, no tombstone exists, and the lock
/// file and state dir are intact.
#[cfg(unix)]
#[tokio::test]
async fn a_live_holder_in_another_process_refuses_deletion_and_changes_nothing() {
    use std::process::Command;

    struct KillOnDrop(std::process::Child);
    impl Drop for KillOnDrop {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    let root = temp_root("c-live-holder");
    let sessions = root.join("sessions");
    let store = FsSessionStore::new(&sessions);
    let id = "c-session";
    save_session(&store, id).await;
    let dir = sessions.join(id);
    std::fs::create_dir_all(&dir).unwrap();

    let child = KillOnDrop(Command::new("sleep").arg("30").spawn().unwrap());
    let pid = child.0.id();
    std::fs::write(dir.join("lock"), format!("{pid}\n")).unwrap();

    let refused = delete_session(&store, &sessions, id).unwrap_err();
    match refused {
        HarnessError::LockHeld { pid: holder } => assert_eq!(holder, pid),
        other => panic!("expected LockHeld, got {other:?}"),
    }
    assert!(store.load(id).await.unwrap().is_some());
    assert!(!store.is_deleted(id).unwrap());
    assert!(dir.join("lock").is_file());
    assert!(dir.is_dir());

    drop(child);
    let _ = std::fs::remove_dir_all(root);
}

/// d. Deletion removes the session record, the per-session state dir with
/// everything in it, and the tombstone, reports `Complete`, and leaves
/// independent state beside the sessions tree untouched. A retry
/// converges and removes nothing else.
#[tokio::test]
async fn delete_session_removes_the_session_and_reports_complete() {
    let root = temp_root("d-complete");
    let sessions = root.join("sessions");
    let store = FsSessionStore::new(&sessions);
    let id = "d-session";
    save_session(&store, id).await;
    let dir = sessions.join(id);
    std::fs::create_dir_all(&dir).unwrap();
    // Per-session engine state inside the removable tree.
    std::fs::write(dir.join("scratch.duckdb"), b"scratch").unwrap();
    std::fs::write(dir.join("journal.log"), b"journal").unwrap();
    // Independent state beside the sessions tree: the app-level store.
    let state_db = root.join("state.sqlite3");
    std::fs::write(&state_db, b"state").unwrap();

    let status = delete_session(&store, &sessions, id).unwrap();
    assert_eq!(status, DeleteStatus::Complete);
    assert!(!sessions.join(format!("{id}.json")).exists());
    assert!(!dir.exists());
    assert!(!store.is_deleted(id).unwrap());
    assert!(store.pending_deletions().unwrap().is_empty());
    assert!(state_db.is_file());
    assert!(store.load(id).await.unwrap().is_none());

    let again = delete_session(&store, &sessions, id).unwrap();
    assert_eq!(again, DeleteStatus::Complete);
    assert!(state_db.is_file());
    assert!(!dir.exists());
    assert!(store.pending_deletions().unwrap().is_empty());

    let _ = std::fs::remove_dir_all(root);
}
