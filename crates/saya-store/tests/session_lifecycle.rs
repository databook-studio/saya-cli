//! Capacity reporting for filesystem session records: the public-API
//! contract is that a capacity report agrees byte-for-byte with what a
//! save would emit (same redaction, same continuation preparation, same
//! pretty serialization), never writes anything, and never materializes
//! the record into a buffer. Boundaries: below 3 MiB is `Ok`, from the
//! warning threshold to the 4 MiB ceiling is `Warning`, past the ceiling
//! is `OverCap` — and a save of an over-cap record is refused, exactly
//! as before, with the previous record untouched.

use saya_store::{
    FsSessionStore, MAX_SESSION_BYTES, RedactedMessage, RedactedSession, SESSION_WARN_BYTES,
    SessionCapacityStatus, SessionStore, StoreError, StoredContinuation, redact,
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
