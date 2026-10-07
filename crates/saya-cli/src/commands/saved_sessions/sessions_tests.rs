//! Unit tests for the shared saved-session operations: the listing shape
//! (order, sizes, warning-band label, pending names, bound), and every
//! delete outcome — including the file-system effects each one must and
//! must not have.

use super::*;
use saya_store::{
    FsSessionStore, RedactedMessage, RedactedSession, SESSION_WARN_BYTES, SessionCapacityStatus,
    SessionStore,
};
use std::path::PathBuf;
use std::time::Duration;

fn block_on<T>(future: impl Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime")
        .block_on(future)
}

fn scratch(label: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!(
        "saya-cli-sessions-unit-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("sessions")).unwrap();
    // The tombstone lives in the store root's PARENT (`<root>/../lifecycle`),
    // so a store root that is a direct child of the temp dir shares ONE
    // lifecycle directory with every other such test — a parallel delete's
    // tombstone appeared in another test's "empty store" listing. Hand back
    // the nested `sessions` dir: the base then owns `lifecycle/` and
    // `state/` for this store root alone.
    base.join("sessions")
}

fn dir_names(path: &std::path::Path) -> Vec<String> {
    let mut names = std::fs::read_dir(path)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect::<Vec<_>>();
    names.sort();
    names
}

fn pinned_mtime(path: &std::path::Path, ago: Duration) {
    let mtime = std::time::SystemTime::now().checked_sub(ago).unwrap();
    let file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    file.set_modified(mtime).unwrap();
}

struct RestorePermissions(std::path::PathBuf);

impl Drop for RestorePermissions {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o700));
        }
    }
}

#[test]
fn listing_labels_ok_and_near_limit_at_the_warning_band() {
    let sessions = scratch("labels");
    let base = sessions.parent().unwrap().to_path_buf();
    let store = FsSessionStore::new(&sessions);
    let small = RedactedSession {
        id: "small".into(),
        messages: vec![RedactedMessage {
            role: "user".into(),
            content: "hi".into(),
        }],
        ..Default::default()
    };
    let big = RedactedSession {
        id: "big".into(),
        messages: vec![RedactedMessage {
            role: "user".into(),
            content: "x".repeat(SESSION_WARN_BYTES),
        }],
        ..Default::default()
    };
    block_on(store.save(small)).unwrap();
    block_on(store.save(big)).unwrap();
    let listing = block_on(list_sessions(&store)).unwrap();
    assert_eq!(
        listing.rows.len(),
        2,
        "{:?}",
        listing
            .rows
            .iter()
            .map(|row| (&row.id, row.size_bytes, row.status))
            .collect::<Vec<_>>()
    );
    // The oversized record sits at or past the store's warning band, the
    // small one below it — both still listed, each labelled from the
    // store's own capacity classification.
    let big_row = listing.rows.iter().find(|row| row.id == "big").unwrap();
    let small_row = listing.rows.iter().find(|row| row.id == "small").unwrap();
    assert_eq!(big_row.status, SessionCapacityStatus::Warning);
    assert!(big_row.size_bytes >= SESSION_WARN_BYTES);
    assert_eq!(small_row.status, SessionCapacityStatus::Ok);
    let text = render_listing(&listing, super::operations::unix_ms_now(), &sessions);
    let big_line = text.lines().find(|line| line.starts_with("big\t")).unwrap();
    let small_line = text
        .lines()
        .find(|line| line.starts_with("small\t"))
        .unwrap();
    assert!(big_line.contains("near-limit"), "{big_line}");
    assert!(small_line.contains("ok"), "{small_line}");
    assert!(small_line.contains("KiB"), "{small_line}");
    assert!(text.ends_with(&format!("Storage root: {}", sessions.display())));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn listing_rows_are_newest_first_with_ages() {
    let sessions = scratch("order");
    let base = sessions.parent().unwrap().to_path_buf();
    std::fs::write(sessions.join("older.json"), "{}").unwrap();
    std::fs::write(sessions.join("newer.json"), "{}").unwrap();
    pinned_mtime(&sessions.join("older.json"), Duration::from_secs(30));
    let store = FsSessionStore::new(&sessions);
    let listing = block_on(list_sessions(&store)).unwrap();
    assert_eq!(
        listing
            .rows
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>(),
        vec!["newer", "older"]
    );
    // Ages come from the modification stamp at render time: a 90-second
    // age renders as one minute.
    let newest_ms = listing.rows[0].modified_unix_ms;
    let text = render_listing(&listing, newest_ms + 90_000, &sessions);
    let newest_line = text
        .lines()
        .find(|line| line.starts_with("newer\t"))
        .unwrap();
    assert!(newest_line.ends_with("1m ago"), "{newest_line}");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn pending_deletions_are_listed_separately() {
    let sessions = scratch("pending");
    let base = sessions.parent().unwrap().to_path_buf();
    std::fs::write(sessions.join("a.json"), "{}").unwrap();
    // A pending-deletion tombstone lives outside the sessions tree (the
    // store's rule): `<sessions>.parent()/lifecycle/session-<id>.tombstone`.
    let lifecycle = base.join("lifecycle");
    std::fs::create_dir_all(&lifecycle).unwrap();
    std::fs::write(lifecycle.join("session-b.tombstone"), "{}").unwrap();
    let store = FsSessionStore::new(&sessions);
    let listing = block_on(list_sessions(&store)).unwrap();
    assert_eq!(listing.pending, vec!["b".to_owned()]);
    assert!(
        listing.rows.iter().all(|row| row.id != "b"),
        "a tombstoned session is hidden from the rows"
    );
    let text = render_listing(&listing, 0, &sessions);
    assert!(text.contains("pending deletion: b"), "{text}");
    assert!(!text.contains("pending deletion: a"), "{text}");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn an_empty_directory_lists_no_saved_sessions() {
    let sessions = scratch("empty");
    let base = sessions.parent().unwrap().to_path_buf();
    let store = FsSessionStore::new(&sessions);
    let listing = block_on(list_sessions(&store)).unwrap();
    assert!(listing.rows.is_empty());
    assert!(listing.pending.is_empty());
    assert!(!listing.more);
    let text = render_listing(&listing, 0, &sessions);
    assert!(text.starts_with("No saved sessions."), "{text}");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn listing_pages_up_to_the_bound_and_says_more_beyond() {
    let sessions = scratch("bound");
    let base = sessions.parent().unwrap().to_path_buf();
    for index in 0..1001 {
        std::fs::write(sessions.join(format!("s{index:04}.json")), "{}").unwrap();
    }
    let store = FsSessionStore::new(&sessions);
    let listing = block_on(list_sessions(&store)).unwrap();
    assert_eq!(listing.rows.len(), LISTING_BOUND);
    assert!(listing.more, "1001 sessions must say more exist");
    let text = render_listing(&listing, 0, &sessions);
    assert!(text.contains("(more available)"), "{text}");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn delete_removes_only_that_session_and_its_state() {
    let sessions = scratch("delete");
    let base = sessions.parent().unwrap().to_path_buf();
    std::fs::write(sessions.join("a.json"), "{}").unwrap();
    std::fs::create_dir_all(sessions.join("a")).unwrap();
    std::fs::write(sessions.join("a").join("state.txt"), "x").unwrap();
    std::fs::write(sessions.join("other.json"), "{}").unwrap();
    std::fs::write(base.join("state.sqlite3"), "sentinel").unwrap();
    let store = FsSessionStore::new(&sessions);
    let outcome = delete_saved_session(&store, &sessions, "a", None);
    assert_eq!(outcome, DeleteOutcome::Deleted, "{outcome:?}");
    assert!(!sessions.join("a.json").exists(), "record removed");
    assert!(!sessions.join("a").exists(), "state dir removed");
    assert!(sessions.join("other.json").exists(), "other session kept");
    assert_eq!(
        std::fs::read_to_string(base.join("state.sqlite3")).unwrap(),
        "sentinel",
        "independent data kept"
    );
    let (code, text) = render_delete(&outcome, "a");
    assert_eq!(code, 0);
    assert!(text.contains(RETAINED_NOTE), "{text}");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn a_tombstone_only_leftover_is_cleared_by_delete() {
    let sessions = scratch("ghost");
    let base = sessions.parent().unwrap().to_path_buf();
    let store = FsSessionStore::new(&sessions);
    // Only a tombstone: record and state dir already gone (a crash landed
    // between removal steps, or the tombstone's own removal failed).
    store.begin_delete("ghost").unwrap();
    let listing = block_on(list_sessions(&store)).unwrap();
    assert_eq!(listing.pending, vec!["ghost".to_owned()]);
    assert!(listing.rows.is_empty());
    // The same command finishes the pending deletion: everything the
    // tombstone covers is already gone, so this converges immediately.
    let outcome = delete_saved_session(&store, &sessions, "ghost", None);
    assert_eq!(outcome, DeleteOutcome::Deleted, "{outcome:?}");
    assert!(
        !base
            .join("lifecycle")
            .join("session-ghost.tombstone")
            .exists(),
        "the tombstone is gone"
    );
    assert!(
        store.pending_deletions().unwrap().is_empty(),
        "no pending deletion remains"
    );
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn deleting_an_absent_session_creates_nothing() {
    let sessions = scratch("absent");
    let base = sessions.parent().unwrap().to_path_buf();
    let before = dir_names(&sessions);
    let store = FsSessionStore::new(&sessions);
    let outcome = delete_saved_session(&store, &sessions, "typo-id", None);
    assert_eq!(outcome, DeleteOutcome::NoSuchSession, "{outcome:?}");
    assert_eq!(dir_names(&sessions), before, "nothing created");
    assert!(
        !sessions.join("typo-id").exists(),
        "no transient state dir may appear for a typo"
    );
    assert!(!base.join("lifecycle").exists(), "no tombstone was written");
    let (code, text) = render_delete(&outcome, "typo-id");
    assert_eq!(code, EXIT_SESSIONS_ERROR);
    assert!(text.contains("no saved session"), "{text}");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn invalid_ids_are_refused_without_filesystem_effect() {
    let sessions = scratch("invalid");
    let base = sessions.parent().unwrap().to_path_buf();
    let before = dir_names(&sessions);
    let store = FsSessionStore::new(&sessions);
    for id in ["../x", "a/b", ""] {
        let outcome = delete_saved_session(&store, &sessions, id, None);
        assert_eq!(outcome, DeleteOutcome::InvalidId, "{id:?}");
        assert_eq!(dir_names(&sessions), before, "{id:?} changed nothing");
        assert!(!base.join("x.json").exists(), "{id:?} stayed inside");
        assert!(
            !base.join("lifecycle").exists(),
            "{id:?} wrote no tombstone"
        );
    }
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn the_current_session_is_refused_without_touching_data() {
    let sessions = scratch("current");
    let base = sessions.parent().unwrap().to_path_buf();
    std::fs::write(sessions.join("a.json"), "{}").unwrap();
    std::fs::create_dir_all(sessions.join("a")).unwrap();
    let store = FsSessionStore::new(&sessions);
    let outcome = delete_saved_session(&store, &sessions, "a", Some("a"));
    assert_eq!(outcome, DeleteOutcome::CurrentSession, "{outcome:?}");
    assert!(sessions.join("a.json").exists(), "data untouched");
    assert!(sessions.join("a").exists(), "state dir untouched");
    assert!(!base.join("lifecycle").exists(), "no tombstone was written");
    let (code, text) = render_delete(&outcome, "a");
    assert_eq!(code, EXIT_SESSIONS_ERROR);
    assert!(text.contains("live session"), "{text}");
    let _ = std::fs::remove_dir_all(base);
}

#[cfg(unix)]
#[test]
fn a_live_session_is_refused_and_left_intact() {
    let sessions = scratch("live");
    let base = sessions.parent().unwrap().to_path_buf();
    std::fs::write(sessions.join("a.json"), "{}").unwrap();
    std::fs::create_dir_all(sessions.join("a")).unwrap();
    let holder = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    struct KillGuard(std::process::Child);
    impl Drop for KillGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let pid = holder.id();
    let _guard = KillGuard(holder);
    std::fs::write(sessions.join("a").join("lock"), format!("{pid}\n")).unwrap();
    let store = FsSessionStore::new(&sessions);
    let outcome = delete_saved_session(&store, &sessions, "a", None);
    assert_eq!(outcome, DeleteOutcome::Live { pid }, "{outcome:?}");
    assert!(sessions.join("a.json").exists(), "data intact");
    assert!(
        !base.join("lifecycle").exists(),
        "no tombstone while the holder lives"
    );
    let (code, text) = render_delete(&outcome, "a");
    assert_eq!(code, EXIT_SESSIONS_ERROR);
    assert!(text.contains(&pid.to_string()), "{text}");
    let _ = std::fs::remove_dir_all(base);
}

#[cfg(unix)]
#[test]
fn pending_cleanup_is_reported_and_a_retry_converges() {
    use std::os::unix::fs::PermissionsExt;
    let sessions = scratch("pending-cleanup");
    let base = sessions.parent().unwrap().to_path_buf();
    std::fs::write(sessions.join("a.json"), "{}").unwrap();
    let nested = sessions.join("a").join("nested");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(nested.join("keep"), "x").unwrap();
    // Read+execute, no write: children inside cannot be unlinked, so the
    // cleanup cannot finish on this attempt.
    std::fs::set_permissions(&nested, std::fs::Permissions::from_mode(0o500)).unwrap();
    let _restore = RestorePermissions(nested.clone());
    let store = FsSessionStore::new(&sessions);
    let outcome = delete_saved_session(&store, &sessions, "a", None);
    match &outcome {
        DeleteOutcome::Pending { remaining } => assert!(*remaining >= 1, "{outcome:?}"),
        other => panic!("expected Pending, got {other:?}"),
    }
    assert!(
        base.join("lifecycle").join("session-a.tombstone").exists(),
        "the tombstone is durable outside the sessions tree"
    );
    // Permissions restored: the same command converges.
    std::fs::set_permissions(&nested, std::fs::Permissions::from_mode(0o700)).unwrap();
    let outcome = delete_saved_session(&store, &sessions, "a", None);
    assert_eq!(outcome, DeleteOutcome::Deleted, "{outcome:?}");
    assert!(!sessions.join("a").exists());
    assert!(!sessions.join("a.json").exists());
    assert!(
        !base.join("lifecycle").join("session-a.tombstone").exists(),
        "a complete delete removes the tombstone"
    );
    let _ = std::fs::remove_dir_all(base);
}

/// Two store roots handed out by the helper must own different lifecycle
/// parents: tombstones live in `<store root>/../lifecycle`, so roots that
/// share a parent share every tombstone — a concurrent delete test's
/// tombstone leaked into another test's "empty store" listing. A tombstone
/// begun through one root stays invisible to the other.
#[test]
fn two_store_roots_from_the_helper_keep_their_tombstones_apart() {
    let root_a = scratch("isolate-a");
    let root_b = scratch("isolate-b");
    let parent_a = root_a.parent().expect("scratch parent").to_path_buf();
    let parent_b = root_b.parent().expect("scratch parent").to_path_buf();
    assert_ne!(
        parent_a, parent_b,
        "two scratch store roots share a lifecycle parent: {parent_a:?}"
    );
    assert_ne!(
        parent_a.join("lifecycle"),
        parent_b.join("lifecycle"),
        "the tombstone directories must be distinct paths"
    );
    // A tombstone-only delete begun through root A (no record, no live
    // lock: the tombstone survives) is root A's pending deletion alone.
    let store_a = FsSessionStore::new(&root_a);
    store_a.begin_delete("a").expect("the tombstone is written");
    assert_eq!(
        block_on(list_sessions(&store_a)).unwrap().pending,
        vec!["a".to_owned()],
        "root A sees its own tombstone"
    );
    let store_b = FsSessionStore::new(&root_b);
    assert!(
        block_on(list_sessions(&store_b))
            .unwrap()
            .pending
            .is_empty(),
        "root B must not see root A's tombstone"
    );
    let _ = std::fs::remove_dir_all(parent_a);
    let _ = std::fs::remove_dir_all(parent_b);
}
