use saya_store::{
    FsSessionStore, MAX_SESSION_BYTES, RedactedMessage, RedactedSession, RedactedToolMetadata,
    RedactedTurn, SessionHistoryQuery, SessionStore, StoreError, StoredContinuation,
};
use saya_types::{
    CONTINUATION_VERSION, CheckFact, CheckOutcome, FactProvenance, GeneratedNarrative,
    NarrativeSource, SessionContinuationV1, SourceBoundary,
};

fn temp_root(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("saya-store-{label}-{}", std::process::id()))
}

fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}

/// A continuation whose every field passes the contract's validate gate: the
/// shape a well-behaved producer writes.
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

#[path = "sessions/atomicity.rs"]
mod atomicity;
#[path = "sessions/continuation_redaction.rs"]
mod continuation_redaction;
#[path = "sessions/continuation_roundtrip.rs"]
mod continuation_roundtrip;
#[path = "sessions/history.rs"]
mod history;
#[path = "sessions/session_storage.rs"]
mod session_storage;
