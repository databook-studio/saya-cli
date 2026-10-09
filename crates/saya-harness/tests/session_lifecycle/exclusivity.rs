//! Acquisition and deletion never both win, in either order — the ordered
//! interleavings that `RunLock`'s in-process pid lockfile cannot race but
//! that the outcomes pin deterministically.

use super::{save_session, temp_root};
use saya_harness::HarnessError;
use saya_harness::session_lifecycle::{acquire_session_lock, delete_session};
use saya_store::{DeleteStatus, FsSessionStore, SessionStore};

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
