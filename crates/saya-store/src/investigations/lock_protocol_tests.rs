//! Lock-protocol tests (Y2): the repository lock is an OS advisory
//! exclusive file lock on `<root>/.lock`. A live holder — however long it
//! pauses, or however abandoned the lock file looks on disk — is never
//! displaced; the guard's drop releases the lock; lock code never deletes
//! or renames the lock file; and a holder that dies without releasing has
//! its lock released by the operating system, never by lock code.

use super::lock::acquire;
use crate::StoreError;
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant, SystemTime},
};

/// The uncontended budget: every uncontended acquisition here succeeds
/// immediately.
const WAIT: Duration = Duration::from_secs(10);
/// The contended budget for a refusal: long enough to prove genuine
/// retries, short enough to keep tests fast.
const CONTENTION_WAIT: Duration = Duration::from_millis(250);
/// The former staleness horizon (R2): a holder is kept alive past it
/// without sleeping, via a backdated lock file, to prove the age-based
/// displacement path is gone entirely.
const FORMER_STALE_HORIZON: Duration = Duration::from_secs(30);
/// The env var that puts a re-invocation of this test binary in child mode.
const CHILD_ENV: &str = "SAYA_TEST_REPO_LOCK_CHILD";
const CHILD_TEST: &str = "investigations::lock_protocol_tests::two_processes_cannot_hold_the_lock";

fn temp_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "saya-investigation-lock-{label}-{}",
        std::process::id()
    ))
}

fn fresh_root(label: &str) -> PathBuf {
    let root = temp_root(label);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

/// Ages the lock file's modification time past any former staleness
/// threshold without sleeping the holder: the test-sized stand-in for a
/// holder that paused longer than the old protocol allowed.
fn backdate_beyond_any_former_stale_threshold(path: &Path) {
    let past = SystemTime::now() - FORMER_STALE_HORIZON * 2;
    let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    file.set_times(std::fs::FileTimes::new().set_modified(past))
        .unwrap();
}

/// Bounded wait for a sentinel file another process writes.
fn wait_for_sentinel(path: &Path) {
    let deadline = Instant::now() + WAIT;
    loop {
        if path.exists() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn live_owner_is_never_displaced() {
    let root = fresh_root("live-owner");
    let path = root.join(".lock");
    let guard = acquire(&root, WAIT).unwrap();

    // Make the lock look maximally abandoned to any age-based protocol:
    // contents nothing can parse, and a modification time far past the
    // former staleness horizon — while the holder stays alive.
    std::fs::write(&path, b"not lock metadata").unwrap();
    backdate_beyond_any_former_stale_threshold(&path);

    assert!(
        matches!(
            acquire(&root, CONTENTION_WAIT),
            Err(StoreError::Unavailable)
        ),
        "a live holder must never be displaced, however abandoned the lock file looks"
    );
    assert!(
        path.exists(),
        "the refused acquisition must not delete or rename the holder's lock file"
    );

    drop(guard);
    let successor =
        acquire(&root, WAIT).expect("dropping the guard must have released the OS lock");
    drop(successor);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn lock_released_on_drop() {
    let root = fresh_root("released-on-drop");
    let path = root.join(".lock");
    let guard = acquire(&root, WAIT).unwrap();
    assert!(path.exists(), "acquisition claims the lock file");

    drop(guard);
    assert!(
        path.exists(),
        "lock code never deletes or renames the lock file"
    );
    let successor =
        acquire(&root, CONTENTION_WAIT).expect("a dropped guard must have released the OS lock");
    drop(successor);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn two_processes_cannot_hold_the_lock() {
    let root = fresh_root("two-process");

    // Child mode: hold the lock until the parent has tried to take it, then
    // die without releasing — only the OS may release a dead holder's lock.
    if let Ok(child_root) = std::env::var(CHILD_ENV) {
        let child_root = PathBuf::from(child_root);
        let guard = acquire(&child_root, WAIT).expect("the child takes the lock uncontended");
        std::fs::write(child_root.join("child-holding"), b"").unwrap();
        wait_for_sentinel(&child_root.join("parent-attempted"));
        std::mem::forget(guard);
        std::process::exit(0);
    }

    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", CHILD_TEST, "--nocapture"])
        .env(CHILD_ENV, &root)
        .spawn()
        .unwrap();
    wait_for_sentinel(&root.join("child-holding"));

    assert!(
        matches!(
            acquire(&root, CONTENTION_WAIT),
            Err(StoreError::Unavailable)
        ),
        "another process's live holder must be respected"
    );
    std::fs::write(root.join("parent-attempted"), b"").unwrap();
    let status = child.wait().unwrap();
    assert!(
        status.success(),
        "the child must exit while still holding the lock"
    );

    let guard = acquire(&root, WAIT).expect("a dead holder's lock is released by the OS");
    drop(guard);
    let _ = std::fs::remove_dir_all(root);
}
