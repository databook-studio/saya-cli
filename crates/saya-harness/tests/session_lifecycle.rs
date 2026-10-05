//! Session-lifecycle coordination tests: tombstone-checked lock
//! acquisition and lock-coordinated deletion over the store's two-phase
//! tombstone mechanics.

use std::{path::PathBuf, sync::Arc};

use saya_harness::HarnessError;
use saya_harness::session_lifecycle::{acquire_session_lock, delete_session, ensure_not_deleted};
use saya_store::{DeleteStatus, FsSessionStore, RedactedSession, SessionStore};

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

/// a. A tombstoned id is refused by every acquisition entry point, and
/// neither call creates anything under `<sessions>/<id>/`.
#[tokio::test]
async fn a_tombstoned_id_cannot_be_acquired_and_nothing_is_created() {
    let root = temp_root("a-tombstoned");
    let sessions = root.join("sessions");
    let store = FsSessionStore::new(&sessions);
    let id = "a-session";
    store.begin_delete(id).unwrap();

    assert_session_deleted(ensure_not_deleted(&store, id).unwrap_err());
    assert_session_deleted(acquire_session_lock(&store, &sessions.join(id), id).unwrap_err());
    // The harness created nothing: not the state dir, not the lock.
    assert!(!sessions.join(id).exists());
    assert!(store.is_deleted(id).unwrap());

    let _ = std::fs::remove_dir_all(root);
}

/// b (deterministic half). The tombstone can appear after the caller's
/// pre-creation check passed; `acquire_session_lock` re-runs that check
/// first and must refuse before any lock file is created.
#[tokio::test]
async fn the_first_check_refuses_a_tombstone_written_after_it_passed() {
    let root = temp_root("b-first-check");
    let sessions = root.join("sessions");
    let store = FsSessionStore::new(&sessions);
    let id = "b-session";
    let state_dir = sessions.join(id);
    std::fs::create_dir_all(&state_dir).unwrap();
    ensure_not_deleted(&store, id).unwrap();
    store.begin_delete(id).unwrap();

    assert_session_deleted(acquire_session_lock(&store, &state_dir, id).unwrap_err());
    assert!(!state_dir.join("lock").exists());

    let _ = std::fs::remove_dir_all(root);
}

/// b (the AFTER-lock half). A tombstone written while the lock is being
/// claimed — the shape of a concurrent deleter that does not honour the
/// lock — must still be caught by the re-check that runs after
/// acquisition. The window is opened deterministically: a large
/// unparsable lock file makes `RunLock::acquire`'s holder read take about
/// a millisecond, while the writer — released by a flag only after the
/// acquirer has entered — needs tens of microseconds to land its
/// tombstone. The tombstone therefore cannot exist at the pre-check
/// inside `acquire_session_lock` (that runs within microseconds of the
/// release) but does exist by the re-check: any refusal below is
/// specifically the AFTER-lock check firing. Zero refusals across the
/// attempts would mean the re-check is not there.
#[tokio::test]
async fn the_after_lock_check_refuses_a_tombstone_written_mid_claim() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let mut refused = 0u32;
    let mut acquired_early = 0u32;
    for attempt in 0..40 {
        let root = temp_root(&format!("b-midclaim-{attempt}"));
        let sessions = root.join("sessions");
        let store = FsSessionStore::new(&sessions);
        let id = "b-session";
        save_session(&store, id).await;
        let state_dir = sessions.join(id);
        std::fs::create_dir_all(&state_dir).unwrap();
        // A large unparsable holder: stale to `RunLock` (reclaimed), but
        // its read opens a millisecond-wide window inside the claim.
        std::fs::write(state_dir.join("lock"), vec![b'x'; 4 << 20]).unwrap();

        let (go, finished) = (
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(false)),
        );
        let writer = {
            let (go, finished) = (go.clone(), finished.clone());
            let store = store.clone();
            let id = id.to_owned();
            std::thread::spawn(move || {
                while !go.load(Ordering::Acquire) {
                    if finished.load(Ordering::Acquire) {
                        return;
                    }
                    std::hint::spin_loop();
                }
                store.begin_delete(&id).unwrap();
            })
        };
        // Let the writer reach its spin loop: thread startup must not
        // decide when the tombstone lands.
        std::thread::sleep(std::time::Duration::from_millis(2));
        go.store(true, Ordering::Release);
        let attempted = acquire_session_lock(&store, &state_dir, id);
        finished.store(true, Ordering::Release);
        writer.join().unwrap();

        match attempted {
            Ok(guard) => {
                acquired_early += 1;
                drop(guard);
            }
            Err(HarnessError::SessionDeleted { .. }) => refused += 1,
            Err(other) => panic!("unexpected outcome on attempt {attempt}: {other:?}"),
        }
        // Reset for the next attempt: the tombstone converges away.
        store.finish_delete(id).unwrap();
        assert!(!store.is_deleted(id).unwrap());
        let _ = std::fs::remove_dir_all(root);
    }
    println!(
        "the_after_lock_check_refuses_a_tombstone_written_mid_claim: refused={refused} acquired_early={acquired_early}"
    );
    assert!(
        refused > 0,
        "acquisition never observed a tombstone that landed mid-claim in 40 attempts"
    );
}

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

/// e. Acquisition and deletion never both win, in either order.
///
/// The two contenders are ordered deliberately rather than raced on
/// threads: `RunLock`'s claim race cannot tell apart two holders in the
/// SAME process (both renames install the same pid, so both re-reads
/// succeed), so a thread-vs-thread race would assert a mutual exclusion
/// the pid lockfile does not provide in-process. What is binding is the
/// outcome in each order, which these ordered interleavings prove
/// deterministically:
///
/// - acquirer first: the guard is held for the whole deletion attempt, so
///   `delete_session` must return `LockHeld` naming this process and
///   change nothing; after release, deletion proceeds and converges.
/// - deleter first: deletion completes and converges, and the lock can
///   never be re-armed on the per-session state dir it removed — the
///   deleted id is not acquirable through the old path.
#[tokio::test]
async fn acquisition_and_deletion_never_both_win() {
    let mut acquirer_won = 0u32;
    let mut deletion_won = 0u32;
    for iteration in 0..10 {
        let root = temp_root(&format!("e-xor-{iteration}"));
        let sessions = root.join("sessions");
        let store = FsSessionStore::new(&sessions);
        let id = "e-session";
        save_session(&store, id).await;
        let state_dir = sessions.join(id);
        if iteration % 2 == 0 {
            std::fs::create_dir_all(&state_dir).unwrap();
            let guard = acquire_session_lock(&store, &state_dir, id).unwrap();
            let refused = delete_session(&store, &sessions, id).unwrap_err();
            match refused {
                HarnessError::LockHeld { pid } => assert_eq!(pid, std::process::id()),
                other => panic!("expected LockHeld, got {other:?}"),
            }
            assert!(store.load(id).await.unwrap().is_some());
            assert!(!store.is_deleted(id).unwrap());
            assert!(state_dir.is_dir());
            acquirer_won += 1;
            drop(guard);
            // Released: deletion proceeds and converges.
            let status = delete_session(&store, &sessions, id).unwrap();
            assert_eq!(status, DeleteStatus::Complete);
        } else {
            let status = delete_session(&store, &sessions, id).unwrap();
            assert_eq!(status, DeleteStatus::Complete);
            let attempted = acquire_session_lock(&store, &state_dir, id).unwrap_err();
            assert!(
                matches!(attempted, HarnessError::Io { .. }),
                "expected Io (state dir removed by the deletion), got {attempted:?}"
            );
            deletion_won += 1;
        }
        // Convergence in both orderings, and deletion stays idempotent.
        assert!(!sessions.join(format!("{id}.json")).exists());
        assert!(!store.is_deleted(id).unwrap());
        assert!(store.pending_deletions().unwrap().is_empty());
        let status = delete_session(&store, &sessions, id).unwrap();
        assert_eq!(status, DeleteStatus::Complete);
        let _ = std::fs::remove_dir_all(root);
    }
    println!(
        "acquisition_and_deletion_never_both_win: acquirer_won={acquirer_won} deletion_won={deletion_won}"
    );
    assert!(acquirer_won > 0 && deletion_won > 0);
}

/// f. Invalid ids are refused everywhere and touch nothing.
#[tokio::test]
async fn an_invalid_id_is_refused_everywhere() {
    let root = temp_root("f-invalid");
    let sessions = root.join("sessions");
    let store = FsSessionStore::new(&sessions);
    for id in ["../x", "a/b", ""] {
        for error in [
            ensure_not_deleted(&store, id).unwrap_err(),
            acquire_session_lock(&store, &sessions.join(id), id).unwrap_err(),
            delete_session(&store, &sessions, id).unwrap_err(),
        ] {
            assert!(
                matches!(error, HarnessError::Store { .. }),
                "expected Store for {id:?}, got {error:?}"
            );
        }
    }
    // Nothing was created: not the sessions root, not a tombstone dir.
    assert!(!sessions.exists());

    let _ = std::fs::remove_dir_all(root);
}
