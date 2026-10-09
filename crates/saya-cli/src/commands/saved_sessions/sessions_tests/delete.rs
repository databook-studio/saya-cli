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
