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

#[path = "session_lifecycle/capacity.rs"]
mod capacity;
#[path = "session_lifecycle/deletion.rs"]
mod deletion;
#[path = "session_lifecycle/deletion_recovery.rs"]
mod deletion_recovery;
#[path = "session_lifecycle/successor_publish.rs"]
mod successor_publish;
#[path = "session_lifecycle/successor_validation.rs"]
mod successor_validation;
