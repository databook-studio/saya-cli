//! Resume-time restoration of a saved session's continuation: the round
//! trip through the real store, every rejection reason, and the guarantees
//! around them — a rejected record is never dropped, and the runtime privacy
//! gate still governs what a restored summary feeds.

use super::{RejectReason, RestoreOutcome, restore_continuation};
use crate::interactive::session::continuation::compaction::manual::{
    apply_with_continuation, plan as compact_plan,
};
use crate::interactive::session::continuation::record::source_digest;
use crate::interactive::session::lifecycle::resume::{
    SessionDefaults, block_on, state_from_redacted,
};
use crate::interactive::session::state::SessionState;
use saya_store::{FsSessionStore, SessionStore, StoredContinuation};
use saya_types::{
    CONTINUATION_VERSION, GeneratedNarrative, NarrativeSource, SessionContinuationV1,
    SourceBoundary, redact,
};

const SUMMARY: &str = "older turns discussed orders";

fn turns(count: usize) -> SessionState {
    let mut state = SessionState::new("s1", None, "m");
    for index in 0..count {
        state.record_turn(format!("q{index}"), format!("a{index}"), false, Vec::new());
    }
    state
}

fn defaults() -> SessionDefaults {
    SessionDefaults {
        provider: "ollama".into(),
        model: "m".into(),
        allow_data_sharing: true,
        approval_mode: "ask".into(),
    }
}

/// Compacts `state` through the production path: the plan plus
/// `apply_with_continuation` with the test digest resolver (no workspace
/// files to resolve).
fn compact(state: &mut SessionState, summary: &str) {
    let history = state.provider_history();
    let plan = compact_plan(&state.turns, &history).expect("a long history plans");
    apply_with_continuation(state, &plan, summary, &|_| None).expect("the summary applies");
}

fn store_root(label: &str) -> std::path::PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-restore-{label}-{}-{stamp}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create the store root");
    root
}

/// Saves `state` through the real store and resumes it through the one path
/// every resume goes through: `state_from_redacted`.
fn save_and_resume(root: &std::path::Path, state: &SessionState) -> SessionState {
    let store = FsSessionStore::new(root);
    block_on(store.save(state.redacted())).expect("the record saves");
    let loaded = block_on(store.load("s1"))
        .expect("loads")
        .expect("the record is there");
    state_from_redacted(loaded, &defaults())
}

/// A rejected continuation leaves the session exactly as any other resume:
/// no summary, no compacted turns, every persisted turn replayed.
fn assert_resumed_unchanged(state: &SessionState, turn_count: usize) {
    assert!(state.compaction_summary.is_none(), "no summary is restored");
    assert_eq!(state.compacted_turns, 0, "no turns sit behind a summary");
    let replayed = state.provider_request_history();
    assert_eq!(
        replayed.len(),
        turn_count * 2,
        "every persisted turn replays: {replayed:?}"
    );
    assert_eq!(
        replayed[replayed.len() - 1].content,
        format!("a{}", turn_count - 1)
    );
}

include!("restore_tests/successful_restore.rs");

include!("restore_tests/stale_and_invalid.rs");

include!("restore_tests/rejection_and_privacy.rs");

include!("restore_tests/carried_records.rs");
include!("restore_tests/legacy_restore.rs");
