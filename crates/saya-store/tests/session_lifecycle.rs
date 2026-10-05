//! Capacity reporting for filesystem session records: the public-API
//! contract is that a capacity report agrees byte-for-byte with what a
//! save would emit (same redaction, same continuation preparation, same
//! pretty serialization), never writes anything, and never materializes
//! the record into a buffer. Boundaries: below 3 MiB is `Ok`, from the
//! warning threshold to the 4 MiB ceiling is `Warning`, past the ceiling
//! is `OverCap` — and a save of an over-cap record is refused, exactly
//! as before, with the previous record untouched.
//!
//! The deletion section covers two-phase session deletion: `begin_delete`
//! writes a durable tombstone outside the sessions tree that hides the
//! session at once and refuses saves; `finish_delete` removes the session
//! record, its per-session state directory, staged temps, and quarantined
//! copies — never following a symlink, never touching data outside the
//! id's own paths — reporting `Pending` with the tombstone kept until
//! every removal succeeds, so a retry converges.

use saya_store::{
    DeleteStatus, FsSessionStore, MAX_SESSION_BYTES, ParentLink, RedactedMessage, RedactedSession,
    SESSION_WARN_BYTES, SessionCapacityStatus, SessionHistoryQuery, SessionStore, StoreError,
    StoredContinuation, SuccessorOutcome, redact,
};
use saya_types::{
    CONTINUATION_VERSION, CheckFact, CheckOutcome, FactProvenance, GeneratedNarrative,
    NarrativeSource, SessionContinuationV1, SourceBoundary,
};

fn temp_root(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "saya-store-capacity-{label}-{}",
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

/// The bytes a save would emit for this record, computed in the test by
/// applying the same redaction the save does, then pretty-serializing.
/// This is the test's own oracle: the production capacity path is never
/// consulted for the expectation.
fn save_emitted_bytes(session: &RedactedSession) -> usize {
    let mut expected = session.clone();
    for message in &mut expected.messages {
        message.content = redact(&message.content);
    }
    for turn in &mut expected.turns {
        turn.user = redact(&turn.user);
        turn.assistant = redact(&turn.assistant);
    }
    serde_json::to_vec_pretty(&expected).unwrap().len()
}

/// A one-message record whose emitted pretty JSON lands at exactly
/// `target` bytes. The fixed overhead is measured with a probe record
/// carrying the SAME id — the id is part of the JSON — and plain `x`
/// runs pass through redaction unchanged, so the payload makes up the
/// difference exactly.
fn session_emitting(id: &str, target: usize) -> RedactedSession {
    let probe = message_session(id, "x");
    let overhead = save_emitted_bytes(&probe) - "x".len();
    let content_len = target - overhead;
    assert!(
        content_len > 0,
        "target {target} is inside the record overhead {overhead}"
    );
    message_session(id, &"x".repeat(content_len))
}

#[test]
fn a_small_record_reports_ok_with_exact_used_bytes() {
    let root = temp_root("ok");
    let _ = std::fs::remove_dir_all(&root);
    let store = FsSessionStore::new(&root);
    let session = message_session("small", "hello password=hunter2 world");
    let unredacted_len = serde_json::to_vec_pretty(&session).unwrap().len();

    let capacity = store.capacity_of(session.clone()).unwrap();

    assert_eq!(
        capacity.used_bytes,
        save_emitted_bytes(&session),
        "used_bytes must equal the pretty-JSON length of the redacted record"
    );
    assert_ne!(
        capacity.used_bytes, unredacted_len,
        "redaction must be part of the accounting"
    );
    assert_eq!(capacity.status, SessionCapacityStatus::Ok);
    assert_eq!(capacity.warn_bytes, SESSION_WARN_BYTES);
    assert_eq!(capacity.max_bytes, MAX_SESSION_BYTES);
    assert!(capacity.used_bytes < SESSION_WARN_BYTES);
    assert!(!root.exists(), "capacity_of must not create the store root");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn the_warning_boundary_is_exact_at_the_threshold() {
    assert_eq!(
        SESSION_WARN_BYTES,
        3 << 20,
        "the warning band starts at 3 MiB, one MiB below the 4 MiB save ceiling"
    );
    let root = temp_root("warn");
    let _ = std::fs::remove_dir_all(&root);
    let store = FsSessionStore::new(&root);

    let at_warn = session_emitting("warn", SESSION_WARN_BYTES);
    let capacity = store.capacity_of(at_warn).unwrap();
    assert_eq!(capacity.used_bytes, SESSION_WARN_BYTES);
    assert_eq!(capacity.status, SessionCapacityStatus::Warning);

    let below_warn = session_emitting("below", SESSION_WARN_BYTES - 1);
    let capacity = store.capacity_of(below_warn).unwrap();
    assert_eq!(capacity.used_bytes, SESSION_WARN_BYTES - 1);
    assert_eq!(capacity.status, SessionCapacityStatus::Ok);
    assert!(!root.exists(), "capacity_of must not create the store root");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn at_the_ceiling_the_capacity_warns_and_a_save_still_succeeds() {
    let root = temp_root("ceiling");
    let _ = std::fs::remove_dir_all(&root);
    let store = FsSessionStore::new(&root);

    let at_max = session_emitting("at-max", MAX_SESSION_BYTES);
    let capacity = store.capacity_of(at_max.clone()).unwrap();
    assert_eq!(capacity.used_bytes, MAX_SESSION_BYTES);
    assert_eq!(capacity.status, SessionCapacityStatus::Warning);

    block_on(store.save(at_max)).unwrap();
    let saved = std::fs::metadata(root.join("at-max.json")).unwrap().len() as usize;
    assert_eq!(
        saved, MAX_SESSION_BYTES,
        "the on-disk record is exactly the ceiling"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn over_the_ceiling_capacity_is_overcap_and_a_save_is_refused() {
    let root = temp_root("over-cap");
    let _ = std::fs::remove_dir_all(&root);
    let store = FsSessionStore::new(&root);
    block_on(store.save(message_session("bounded", "old"))).unwrap();
    let path = root.join("bounded.json");
    let before = std::fs::read(&path).unwrap();

    let over = session_emitting("bounded", MAX_SESSION_BYTES + 1);
    let capacity = store.capacity_of(over.clone()).unwrap();
    assert_eq!(
        capacity.used_bytes,
        MAX_SESSION_BYTES + 1,
        "over-cap reports more than the ceiling, not a count past the bound"
    );
    assert_eq!(capacity.status, SessionCapacityStatus::OverCap);

    let error = block_on(store.save(over)).expect_err("an over-cap save must be refused");
    assert_eq!(error, StoreError::LimitExceeded);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "a refused save must leave the previous bytes untouched"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn stored_capacity_reports_the_saved_file_length() {
    let root = temp_root("stored");
    let _ = std::fs::remove_dir_all(&root);
    let store = FsSessionStore::new(&root);
    let session = message_session("stored", "password=hunter2 and more text");
    block_on(store.save(session.clone())).unwrap();

    let capacity = store
        .stored_capacity("stored")
        .unwrap()
        .expect("the record exists");
    let on_disk = std::fs::metadata(root.join("stored.json")).unwrap().len() as usize;
    assert_eq!(
        capacity.used_bytes, on_disk,
        "the stored length is exactly what the save emitted"
    );
    assert_eq!(capacity.used_bytes, save_emitted_bytes(&session));
    assert_eq!(capacity.status, SessionCapacityStatus::Ok);

    assert!(store.stored_capacity("missing").unwrap().is_none());
    assert_eq!(
        store.stored_capacity("../x").unwrap_err(),
        StoreError::Unavailable,
        "an invalid id gives the same error the path check gives a save"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn an_oversize_stored_file_reports_overcap_with_its_real_length() {
    let root = temp_root("stored-over");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("huge.json"), "x".repeat(MAX_SESSION_BYTES + 1)).unwrap();
    let store = FsSessionStore::new(&root);

    let capacity = store
        .stored_capacity("huge")
        .unwrap()
        .expect("the file exists");
    assert_eq!(capacity.used_bytes, MAX_SESSION_BYTES + 1);
    assert_eq!(capacity.status, SessionCapacityStatus::OverCap);
    let _ = std::fs::remove_dir_all(root);
}

fn safe_continuation(narrative: &str, notes: &str, label: &str) -> SessionContinuationV1 {
    SessionContinuationV1 {
        version: CONTINUATION_VERSION,
        source: SourceBoundary {
            through_turn: 3,
            source_sha256: "a".repeat(64),
            workspace_root: None,
            profile: None,
        },
        narrative: Some(GeneratedNarrative {
            text: narrative.into(),
            source: NarrativeSource::Summarizer,
        }),
        user_notes: Some(notes.into()),
        files: vec![],
        checks: vec![CheckFact {
            label: label.into(),
            outcome: CheckOutcome::Passed,
            observed_unix_ms: 1_000,
            provenance: FactProvenance::ToolObserved {
                tool: "bounded_sql_query".into(),
            },
        }],
    }
}

#[test]
fn an_unpreparable_continuation_is_reported_like_a_save_refusal() {
    let root = temp_root("continuation-over");
    let _ = std::fs::remove_dir_all(&root);
    let store = FsSessionStore::new(&root);

    let mut record = safe_continuation("fine", "fine", "cargo test");
    record.narrative = Some(GeneratedNarrative {
        text: "x".repeat(9 * 1024),
        source: NarrativeSource::Deterministic,
    });
    let session = RedactedSession {
        id: "continued".into(),
        continuation: Some(StoredContinuation::Current(record)),
        ..Default::default()
    };
    assert_eq!(
        store.capacity_of(session.clone()).unwrap_err(),
        StoreError::LimitExceeded,
        "a continuation that fails preparation reports the save's error"
    );
    assert_eq!(
        block_on(store.save(session)).unwrap_err(),
        StoreError::LimitExceeded,
        "the capacity error matches what a save of the same record returns"
    );

    let mut record = safe_continuation("fine", "fine", "cargo test");
    record.narrative = Some(GeneratedNarrative {
        text: "\u{7}".into(),
        source: NarrativeSource::Deterministic,
    });
    let session = RedactedSession {
        id: "continued".into(),
        continuation: Some(StoredContinuation::Current(record)),
        ..Default::default()
    };
    assert_eq!(
        store.capacity_of(session.clone()).unwrap_err(),
        StoreError::Invalid
    );
    assert_eq!(
        block_on(store.save(session)).unwrap_err(),
        StoreError::Invalid
    );

    let _ = std::fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------
// Two-phase deletion: tombstone + retryable no-follow cleanup.

/// Builds the store at `<tmp>/sessions` so `<tmp>` is the state dir whose
/// independent contents must survive every deletion.
fn deletion_store(label: &str) -> (std::path::PathBuf, FsSessionStore) {
    let state =
        std::env::temp_dir().join(format!("saya-store-delete-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&state);
    let store = FsSessionStore::new(state.join("sessions"));
    (state, store)
}

/// Plants the state-dir siblings of `sessions/` that a deletion must
/// never touch.
fn plant_state_sentinels(state: &std::path::Path) {
    std::fs::create_dir_all(state.join("runs")).unwrap();
    std::fs::write(state.join("state.sqlite3"), "db").unwrap();
    std::fs::write(state.join("input_history"), "history").unwrap();
    std::fs::write(state.join("runs/step"), "run").unwrap();
}

fn plant_session(store: &FsSessionStore, id: &str, content: &str) {
    block_on(store.save(message_session(id, content))).unwrap();
}

fn history_ids(store: &FsSessionStore, limit: usize) -> Vec<String> {
    block_on(store.history(SessionHistoryQuery::first_page(limit).unwrap()))
        .unwrap()
        .entries
        .into_iter()
        .map(|entry| entry.id)
        .collect()
}

#[test]
fn begin_delete_writes_a_tombstone_outside_the_sessions_tree_and_hides_the_session() {
    let (state, store) = deletion_store("tombstone-hide");
    plant_state_sentinels(&state);
    plant_session(&store, "other", "keep");
    plant_session(&store, "a", "hello");
    let session_path = state.join("sessions/a.json");
    let before = std::fs::read(&session_path).unwrap();

    store.begin_delete("a").unwrap();

    let tombstone = state.join("lifecycle/session-a.tombstone");
    assert!(
        tombstone.is_file(),
        "the tombstone lives under <state>/lifecycle/"
    );
    let sessions = std::fs::read_dir(state.join("sessions")).unwrap();
    assert!(
        sessions.filter_map(Result::ok).all(|entry| entry
            .path()
            .extension()
            .is_none_or(|ext| ext != "tombstone")),
        "no tombstone is named inside <sessions>"
    );

    assert!(store.is_deleted("a").unwrap());
    assert!(
        block_on(store.load("a")).unwrap().is_none(),
        "load hides a tombstoned session at once"
    );
    assert_eq!(history_ids(&store, 10), vec!["other".to_owned()]);
    let recent = block_on(store.most_recent())
        .unwrap()
        .expect("the surviving session is still listed");
    assert_eq!(
        recent.id, "other",
        "most_recent skips the tombstoned session"
    );

    let error = block_on(store.save(message_session("a", "resurrect")))
        .expect_err("a save of a tombstoned session is refused");
    assert_eq!(error, StoreError::Conflict);
    assert_eq!(
        std::fs::read(&session_path).unwrap(),
        before,
        "the previous bytes stay untouched until finish"
    );
    let _ = std::fs::remove_dir_all(state);
}

#[test]
fn finish_delete_removes_the_session_its_state_dir_staged_temps_and_quarantine_and_the_tombstone() {
    let (state, store) = deletion_store("finish-complete");
    plant_state_sentinels(&state);
    plant_session(&store, "a", "hello");
    plant_session(&store, "other", "keep");
    plant_session(&store, "a-2", "keep too");
    let sessions = state.join("sessions");
    std::fs::create_dir_all(sessions.join("a/run_program")).unwrap();
    std::fs::create_dir_all(sessions.join("other")).unwrap();
    std::fs::write(sessions.join("a/journal.ndjson"), "j").unwrap();
    std::fs::write(sessions.join("a/run_program/x.json"), "{}").unwrap();
    std::fs::write(sessions.join("a/scratch.duckdb"), "db").unwrap();
    std::fs::write(sessions.join("other/journal.ndjson"), "j").unwrap();
    std::fs::write(sessions.join(".a.json.12.3.tmp"), "staged").unwrap();
    std::fs::write(sessions.join("a.corrupt-111"), "quarantine").unwrap();
    std::fs::write(sessions.join(".a-2.json.1.1.tmp"), "keep").unwrap();
    std::fs::write(sessions.join("a.corrupt-x"), "keep").unwrap();

    store.begin_delete("a").unwrap();
    assert_eq!(store.pending_deletions().unwrap(), vec!["a".to_owned()]);
    let status = store.finish_delete("a").unwrap();

    assert_eq!(status, DeleteStatus::Complete);
    assert!(!sessions.join("a.json").exists());
    assert!(!sessions.join("a").exists());
    assert!(!sessions.join(".a.json.12.3.tmp").exists());
    assert!(!sessions.join("a.corrupt-111").exists());
    assert!(!state.join("lifecycle/session-a.tombstone").exists());
    assert_eq!(store.pending_deletions().unwrap(), Vec::<String>::new());
    // Independent data — the state dir, other sessions, look-alike names.
    assert_eq!(
        std::fs::read(state.join("state.sqlite3")).unwrap(),
        "db".as_bytes().to_vec()
    );
    assert_eq!(
        std::fs::read(state.join("input_history")).unwrap(),
        "history".as_bytes().to_vec()
    );
    assert_eq!(
        std::fs::read(state.join("runs/step")).unwrap(),
        "run".as_bytes().to_vec()
    );
    assert!(sessions.join("other.json").is_file());
    assert!(sessions.join("other/journal.ndjson").is_file());
    assert!(sessions.join("a-2.json").is_file());
    assert_eq!(
        std::fs::read(sessions.join(".a-2.json.1.1.tmp")).unwrap(),
        "keep".as_bytes().to_vec()
    );
    assert_eq!(
        std::fs::read(sessions.join("a.corrupt-x")).unwrap(),
        "keep".as_bytes().to_vec()
    );
    assert_eq!(block_on(store.load("other")).unwrap().unwrap().id, "other");
    let _ = std::fs::remove_dir_all(state);
}

#[cfg(unix)]
#[test]
fn deleting_never_follows_a_symlink() {
    use std::os::unix::fs::symlink;

    let (state, store) = deletion_store("no-follow");
    plant_state_sentinels(&state);
    let sessions = state.join("sessions");
    let victim = state.join("victim.txt");
    std::fs::write(&victim, "precious").unwrap();

    // A link planted INSIDE the id dir is unlinked as a name; its target
    // keeps its bytes.
    std::fs::create_dir_all(sessions.join("a")).unwrap();
    std::fs::write(sessions.join("a/real.ndjson"), "real").unwrap();
    symlink(&victim, sessions.join("a/link")).unwrap();
    store.begin_delete("a").unwrap();
    assert_eq!(store.finish_delete("a").unwrap(), DeleteStatus::Complete);
    assert!(
        !sessions.join("a").exists(),
        "the id dir including the link is gone"
    );
    assert_eq!(
        std::fs::read(&victim).unwrap(),
        "precious".as_bytes().to_vec(),
        "the link target keeps its bytes"
    );

    // The id dir itself replaced by a symlink: only the link is removed,
    // the outside directory and its contents are intact.
    let outside = state.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("keep.txt"), "keep").unwrap();
    symlink(&outside, sessions.join("b")).unwrap();
    store.begin_delete("b").unwrap();
    assert_eq!(store.finish_delete("b").unwrap(), DeleteStatus::Complete);
    assert!(!sessions.join("b").exists(), "only the link is removed");
    assert!(outside.is_dir());
    assert_eq!(
        std::fs::read(outside.join("keep.txt")).unwrap(),
        "keep".as_bytes().to_vec()
    );
    let _ = std::fs::remove_dir_all(state);
}

#[test]
fn traversal_ids_are_refused() {
    let (state, store) = deletion_store("traversal");
    plant_state_sentinels(&state);
    plant_session(&store, "safe", "here");
    let sentinel = std::fs::read(state.join("state.sqlite3")).unwrap();

    for id in ["../x", "a/b", "a\\b", "", ".", ".."] {
        assert_eq!(
            store.begin_delete(id).unwrap_err(),
            StoreError::Unavailable,
            "begin_delete refuses {id:?}"
        );
        assert_eq!(
            store.finish_delete(id).unwrap_err(),
            StoreError::Unavailable,
            "finish_delete refuses {id:?}"
        );
    }

    assert!(
        !state.join("lifecycle").exists(),
        "no tombstone tree is created for refused ids"
    );
    assert_eq!(store.pending_deletions().unwrap(), Vec::<String>::new());
    assert_eq!(
        std::fs::read(state.join("state.sqlite3")).unwrap(),
        sentinel
    );
    assert!(state.join("sessions/safe.json").is_file());
    let _ = std::fs::remove_dir_all(state);
}

#[cfg(unix)]
#[test]
fn a_partial_cleanup_reports_pending_and_a_retry_converges() {
    use std::os::unix::fs::PermissionsExt;

    let (state, store) = deletion_store("pending-retry");
    plant_session(&store, "a", "hello");
    let sessions = state.join("sessions");
    let stubborn = sessions.join("a/stubborn");
    std::fs::create_dir_all(&stubborn).unwrap();
    std::fs::write(stubborn.join("x.json"), "stuck").unwrap();
    // Restores write access even on a failed assert, so the temp tree
    // stays removable.
    struct RestoreMode(std::path::PathBuf);
    impl Drop for RestoreMode {
        fn drop(&mut self) {
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o700));
        }
    }
    let _guard = RestoreMode(stubborn.clone());
    std::fs::set_permissions(&stubborn, std::fs::Permissions::from_mode(0o500)).unwrap();

    store.begin_delete("a").unwrap();
    let status = store.finish_delete("a").unwrap();

    assert!(
        matches!(status, DeleteStatus::Pending { remaining } if remaining >= 1),
        "a blocked child reports pending: {status:?}"
    );
    assert!(
        state.join("lifecycle/session-a.tombstone").is_file(),
        "the tombstone is kept for retry"
    );
    assert!(store.is_deleted("a").unwrap());
    assert!(block_on(store.load("a")).unwrap().is_none(), "still hidden");

    std::fs::set_permissions(&stubborn, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(store.finish_delete("a").unwrap(), DeleteStatus::Complete);
    assert!(!sessions.join("a/stubborn/x.json").exists());
    assert!(!sessions.join("a").exists());
    assert!(!state.join("lifecycle/session-a.tombstone").exists());
    let _ = std::fs::remove_dir_all(state);
}

#[test]
fn repeated_begin_and_finish_are_idempotent() {
    let (state, store) = deletion_store("idempotent");
    plant_session(&store, "a", "hello");
    plant_session(&store, "b", "more");

    store.begin_delete("a").unwrap();
    store.begin_delete("a").unwrap();
    store.begin_delete("b").unwrap();
    assert_eq!(
        store.pending_deletions().unwrap(),
        vec!["a".to_owned(), "b".to_owned()],
        "pending deletions are sorted"
    );

    assert_eq!(store.finish_delete("a").unwrap(), DeleteStatus::Complete);
    assert_eq!(
        store.finish_delete("a").unwrap(),
        DeleteStatus::Complete,
        "the second finish has no tombstone and removes nothing new"
    );
    assert!(!state.join("sessions/a.json").exists());
    assert!(!state.join("lifecycle/session-a.tombstone").exists());
    assert_eq!(store.pending_deletions().unwrap(), vec!["b".to_owned()]);
    assert_eq!(store.finish_delete("b").unwrap(), DeleteStatus::Complete);
    let _ = std::fs::remove_dir_all(state);
}

#[test]
fn finish_without_a_tombstone_deletes_nothing() {
    let (state, store) = deletion_store("no-tombstone");
    plant_session(&store, "a", "hello");
    let session_path = state.join("sessions/a.json");
    let before = std::fs::read(&session_path).unwrap();

    assert_eq!(store.finish_delete("a").unwrap(), DeleteStatus::Complete);

    assert!(session_path.is_file(), "a bare id removes nothing");
    assert_eq!(std::fs::read(&session_path).unwrap(), before);
    assert!(!store.is_deleted("a").unwrap());
    let _ = std::fs::remove_dir_all(state);
}

#[test]
fn a_corrupt_tombstone_fails_closed() {
    let (state, store) = deletion_store("corrupt-tombstone");
    plant_session(&store, "a", "hello");
    let session_path = state.join("sessions/a.json");
    let before = std::fs::read(&session_path).unwrap();
    let lifecycle = state.join("lifecycle");
    std::fs::create_dir_all(&lifecycle).unwrap();
    std::fs::write(lifecycle.join("session-a.tombstone"), "not json at all").unwrap();

    assert_eq!(store.is_deleted("a").unwrap_err(), StoreError::Invalid);
    assert_eq!(store.finish_delete("a").unwrap_err(), StoreError::Invalid);
    assert_eq!(
        std::fs::read(&session_path).unwrap(),
        before,
        "the session data is untouched by a refused finish"
    );
    let _ = std::fs::remove_dir_all(state);
}

// ---------------------------------------------------------------------
// Session rotation: publishing an explicit successor record that links
// to its parent. The parent record is never written; selection — which
// session becomes active — is the CLI's job and happens only after this
// returns. Repeating the same rotation is idempotent; oversize, foreign,
// and duplicate seeds are refused with both records untouched.

fn successor_link(parent: &str, rotation: &str) -> ParentLink {
    ParentLink {
        id: parent.to_owned(),
        rotation_id: rotation.to_owned(),
    }
}

#[test]
fn a_successor_links_to_its_parent_and_leaves_the_parent_byte_identical() {
    let root = temp_root("successor-link");
    let _ = std::fs::remove_dir_all(&root);
    let store = FsSessionStore::new(&root);
    plant_session(&store, "parent", "original");
    let parent_path = root.join("parent.json");
    let before = std::fs::read(&parent_path).unwrap();

    let outcome = store
        .publish_successor("parent", "rot-1", message_session("child", "fresh"))
        .unwrap();

    assert_eq!(outcome, SuccessorOutcome::Published);
    assert_eq!(
        std::fs::read(&parent_path).unwrap(),
        before,
        "rotation never rewrites the parent record"
    );
    let child = block_on(store.load("child")).unwrap().unwrap();
    assert_eq!(child.parent, Some(successor_link("parent", "rot-1")));
    assert_eq!(
        child.messages[0].content, "fresh",
        "the successor carries only its own seed content, not the parent's"
    );
    assert_eq!(
        block_on(store.load("parent")).unwrap().unwrap().messages[0].content,
        "original",
        "the parent stays fully usable"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn repeating_the_same_rotation_yields_one_child() {
    let root = temp_root("successor-repeat");
    let _ = std::fs::remove_dir_all(&root);
    let store = FsSessionStore::new(&root);
    plant_session(&store, "parent", "original");

    assert_eq!(
        store
            .publish_successor("parent", "rot-1", message_session("child", "first"))
            .unwrap(),
        SuccessorOutcome::Published
    );
    let child_path = root.join("child.json");
    let before = std::fs::read(&child_path).unwrap();

    // The seed is ignored on a repeat — even a materially different one.
    let second = message_session("child", "second seed that differs");
    assert_eq!(
        store.publish_successor("parent", "rot-1", second).unwrap(),
        SuccessorOutcome::AlreadyPublished
    );
    assert_eq!(
        std::fs::read(&child_path).unwrap(),
        before,
        "the stored child stays byte-identical across the retry"
    );
    let mut ids = history_ids(&store, 10);
    ids.sort();
    assert_eq!(
        ids,
        vec!["child".to_owned(), "parent".to_owned()],
        "exactly the parent plus one child"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_different_rotation_over_an_existing_successor_id_conflicts() {
    let root = temp_root("successor-different-rot");
    let _ = std::fs::remove_dir_all(&root);
    let store = FsSessionStore::new(&root);
    plant_session(&store, "parent", "original");
    store
        .publish_successor("parent", "rot-1", message_session("child", "first"))
        .unwrap();
    let child_path = root.join("child.json");
    let before = std::fs::read(&child_path).unwrap();

    let error = store
        .publish_successor("parent", "rot-2", message_session("child", "again"))
        .unwrap_err();

    assert_eq!(error, StoreError::Conflict);
    assert_eq!(
        std::fs::read(&child_path).unwrap(),
        before,
        "the existing child is untouched"
    );
    assert_eq!(
        block_on(store.load("child")).unwrap().unwrap().parent,
        Some(successor_link("parent", "rot-1")),
        "the original rotation link survives"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_plain_session_at_the_successor_id_conflicts() {
    let root = temp_root("successor-plain");
    let _ = std::fs::remove_dir_all(&root);
    let store = FsSessionStore::new(&root);
    plant_session(&store, "parent", "original");
    plant_session(&store, "child", "plain");
    let child_path = root.join("child.json");
    let before = std::fs::read(&child_path).unwrap();

    let error = store
        .publish_successor(
            "parent",
            "rot-1",
            message_session("child", "would overwrite"),
        )
        .unwrap_err();

    assert_eq!(error, StoreError::Conflict);
    assert_eq!(
        std::fs::read(&child_path).unwrap(),
        before,
        "a plain session is never replaced by a rotation"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn an_oversize_seed_is_refused_and_both_records_survive() {
    let root = temp_root("successor-oversize");
    let _ = std::fs::remove_dir_all(&root);
    let store = FsSessionStore::new(&root);
    plant_session(&store, "parent", "original");
    let parent_path = root.join("parent.json");
    let before = std::fs::read(&parent_path).unwrap();

    let error = store
        .publish_successor(
            "parent",
            "rot-1",
            session_emitting("child", MAX_SESSION_BYTES + 1),
        )
        .unwrap_err();

    assert_eq!(error, StoreError::LimitExceeded);
    assert_eq!(
        std::fs::read(&parent_path).unwrap(),
        before,
        "the parent record is untouched"
    );
    assert!(!root.join("child.json").exists(), "no successor is written");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_missing_or_corrupt_parent_is_refused() {
    let root = temp_root("successor-parent-missing");
    let _ = std::fs::remove_dir_all(&root);
    let store = FsSessionStore::new(&root);

    let error = store
        .publish_successor("ghost", "rot-1", message_session("child", "x"))
        .unwrap_err();
    assert_eq!(error, StoreError::NotFound, "a missing parent is refused");
    assert!(!root.join("child.json").exists());
    assert!(!root.exists(), "nothing is created for a refused rotation");

    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("broken.json"), "not json at all").unwrap();
    let error = store
        .publish_successor("broken", "rot-1", message_session("child2", "x"))
        .unwrap_err();
    assert_eq!(
        error,
        StoreError::NotFound,
        "a corrupt parent is not usable (existing load quarantine)"
    );
    assert!(
        !root.join("child2.json").exists(),
        "no successor is written"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn tombstoned_parent_or_successor_is_refused() {
    let (state, store) = deletion_store("successor-tombstones");
    plant_session(&store, "parent", "original");

    // A tombstoned successor id is refused even without a record on disk.
    store.begin_delete("gone").unwrap();
    let error = store
        .publish_successor("parent", "rot-1", message_session("gone", "x"))
        .unwrap_err();
    assert_eq!(error, StoreError::Conflict);
    assert!(!state.join("sessions/gone.json").exists());

    // A tombstoned parent is refused; nothing is written for the child.
    store.begin_delete("parent").unwrap();
    let error = store
        .publish_successor("parent", "rot-1", message_session("child", "x"))
        .unwrap_err();
    assert_eq!(error, StoreError::Conflict);
    assert!(!state.join("sessions/child.json").exists());
    let _ = std::fs::remove_dir_all(state);
}

#[test]
fn invalid_ids_and_self_rotation_are_refused() {
    let root = temp_root("successor-invalid");
    let _ = std::fs::remove_dir_all(&root);
    let store = FsSessionStore::new(&root);
    plant_session(&store, "safe", "here");
    let long_rotation = "r".repeat(65);
    let cases: Vec<(&str, &str, &str)> = vec![
        ("../x", "ok", "rot-1"),
        ("safe", "a/b", "rot-1"),
        ("", "ok", "rot-1"),
        ("safe", "", "rot-1"),
        ("safe", "safe", "rot-1"),
        ("safe", "ok", ""),
        ("safe", "ok", "rot/1"),
        ("safe", "ok", "rot.1"),
        ("safe", "ok", "r ot"),
        ("safe", "ok", long_rotation.as_str()),
        ("safe", "ok", "rôt"),
    ];
    for (parent, child, rotation) in &cases {
        let error = store
            .publish_successor(parent, rotation, message_session(child, "x"))
            .unwrap_err();
        assert_eq!(
            error,
            StoreError::Invalid,
            "refuses {parent:?}/{child:?}/{rotation:?}"
        );
    }

    let entries: Vec<String> = std::fs::read_dir(&root)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        entries,
        vec!["safe.json".to_owned()],
        "refused rotations create nothing"
    );

    // The 64-character boundary itself is a valid rotation id.
    let long_ok = "r".repeat(64);
    assert_eq!(
        store
            .publish_successor("safe", &long_ok, message_session("child64", "x"))
            .unwrap(),
        SuccessorOutcome::Published
    );
    assert_eq!(
        block_on(store.load("child64")).unwrap().unwrap().parent,
        Some(successor_link("safe", &long_ok))
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_seed_naming_a_foreign_parent_is_invalid() {
    let root = temp_root("successor-foreign");
    let _ = std::fs::remove_dir_all(&root);
    let store = FsSessionStore::new(&root);
    plant_session(&store, "parent", "original");

    let mut seed = message_session("child", "fresh");
    seed.parent = Some(successor_link("elsewhere", "rot-1"));
    let error = store
        .publish_successor("parent", "rot-1", seed)
        .unwrap_err();
    assert_eq!(error, StoreError::Invalid, "a foreign seed link is refused");
    assert!(
        !root.join("child.json").exists(),
        "nothing is written for a foreign seed"
    );

    // A seed whose own link matches the requested rotation publishes; the
    // store sets the link itself either way.
    let mut seed = message_session("child", "fresh");
    seed.parent = Some(successor_link("parent", "rot-1"));
    assert_eq!(
        store.publish_successor("parent", "rot-1", seed).unwrap(),
        SuccessorOutcome::Published
    );
    assert_eq!(
        block_on(store.load("child")).unwrap().unwrap().parent,
        Some(successor_link("parent", "rot-1"))
    );
    let _ = std::fs::remove_dir_all(root);
}
