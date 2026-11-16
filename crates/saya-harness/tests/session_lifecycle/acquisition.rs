//! Tombstone-checked lock acquisition: a tombstoned id is refused by
//! every acquisition entry point and creates nothing, the pre-creation
//! check refuses a tombstone written after it passed, and the re-check
//! after acquisition catches a tombstone that landed mid-claim.

use std::sync::Arc;

use super::{assert_session_deleted, save_session, temp_root};
use saya_harness::HarnessError;
use saya_harness::session_lifecycle::{acquire_session_lock, ensure_not_deleted};
use saya_store::FsSessionStore;

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
