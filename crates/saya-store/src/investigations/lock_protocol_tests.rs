//! Lock-protocol hardening tests (A1): release must never remove a
//! successor's fresh claim even when it is byte-identical to the displaced
//! lock, restoring a displaced fresh lock must never overwrite a third
//! writer's claim, and stale aside files must not accumulate.

use super::lock::acquire;
use super::stale::break_if_stale;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The contention budget for these tests: nothing here holds the lock
/// across an acquire, so the budget is never burned.
const WAIT: Duration = Duration::from_secs(10);
/// Longer than any lock in these tests, so live locks are never mistaken
/// for stale ones.
const STALE: Duration = Duration::from_secs(30);

fn temp_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "saya-investigation-lock-{label}-{}",
        std::process::id()
    ))
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

fn lock_bytes(acquired_unix_ms: i64) -> Vec<u8> {
    format!("{{\"pid\":1,\"acquired_unix_ms\":{acquired_unix_ms}}}\n").into_bytes()
}

fn stale_bytes() -> Vec<u8> {
    lock_bytes(now_unix_ms() - 60_000)
}

#[test]
#[cfg(unix)]
fn release_never_removes_a_successor_lock() {
    use std::os::unix::fs::MetadataExt;
    let root = temp_root("successor");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join(".lock");
    let guard = acquire(&root, WAIT, STALE).unwrap();

    // A stale break displaced this guard's lock and a successor claimed a
    // fresh file with byte-identical contents (same pid, same acquisition
    // millisecond) but a new inode: release must leave that claim alone.
    let contents = std::fs::read(&path).unwrap();
    let displaced = std::fs::metadata(&path).unwrap().ino();
    let successor = root.join(".lock.successor");
    std::fs::write(&successor, &contents).unwrap();
    assert_ne!(
        displaced,
        std::fs::metadata(&successor).unwrap().ino(),
        "the successor's claim must be a fresh inode"
    );
    std::fs::rename(&successor, &path).unwrap();

    drop(guard);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        contents,
        "release must not remove a successor's byte-identical lock"
    );

    // Control: once the successor's claim is gone, a guard releasing its
    // own untouched lock still removes it.
    std::fs::remove_file(&path).unwrap();
    let guard = acquire(&root, WAIT, STALE).unwrap();
    drop(guard);
    assert!(!path.exists(), "an untouched lock must still be released");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn stale_restore_never_overwrites_a_new_claim() {
    let root = temp_root("restore");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join(".lock");
    let aside = root.join(".lock.stale-1");
    // The break displaced a fresh lock (the aside), and before the restore
    // a third writer claimed the lock path.
    let displaced = lock_bytes(now_unix_ms() - 5_000);
    let third_writer = lock_bytes(now_unix_ms());
    std::fs::write(&aside, &displaced).unwrap();
    #[cfg(unix)]
    let aside_ino = {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(&aside).unwrap().ino()
    };
    std::fs::write(&path, &third_writer).unwrap();

    let aside_remains = super::stale::restore_displaced(&aside, &path);
    assert!(
        aside_remains,
        "a refused restore must leave the aside file in place"
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        third_writer,
        "the restore must not overwrite the third writer's claim"
    );

    // When the lock path is free, the same restore links the displaced
    // lock back, keeping its contents — and on unix its inode, so the
    // displaced holder's identity-guarded release still works.
    std::fs::remove_file(&path).unwrap();
    let aside_remains = super::stale::restore_displaced(&aside, &path);
    assert!(!aside_remains, "a successful restore must remove the aside");
    assert_eq!(std::fs::read(&path).unwrap(), displaced);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().ino(),
            aside_ino,
            "the restore must hard-link, not copy"
        );
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn stale_aside_files_are_cleaned() {
    let root = temp_root("aside-cleanup");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join(".lock");

    // Direct probe: breaking a stale lock renames it aside — never
    // deletes it — and the aside keeps the broken lock's contents as
    // evidence.
    let stale = stale_bytes();
    std::fs::write(&path, &stale).unwrap();
    let aside = break_if_stale(&path, Duration::from_secs(1))
        .expect("a stale break must leave the lock aside, not delete it");
    assert!(!path.exists(), "the break must free the lock path");
    assert_eq!(
        std::fs::read(&aside).unwrap(),
        stale,
        "the aside must keep the broken lock's contents"
    );
    std::fs::remove_file(&aside).unwrap();

    // The production flow: one acquire that breaks a stale lock and
    // re-claims removes the aside file best-effort on success.
    std::fs::write(&path, stale_bytes()).unwrap();
    let guard = acquire(&root, WAIT, STALE).unwrap();
    drop(guard);
    let asides = std::fs::read_dir(&root)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".lock.stale-")
        })
        .count();
    assert_eq!(
        asides, 0,
        "a successful re-claim must remove the aside file"
    );
    assert!(!path.exists(), "release removes the claimed lock");
    let _ = std::fs::remove_dir_all(root);
}
