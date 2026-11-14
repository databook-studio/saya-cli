//! Tests for the shared `/notes` operation: what it shows, what it stores,
//! what it refuses, and how the updated record round-trips through the
//! store and a later compaction.

use super::*;
use crate::interactive::session::continuation::compaction::manual::{
    apply_with_continuation, plan as compact_plan,
};
use crate::interactive::session::lifecycle::resume::{
    SessionDefaults, block_on, state_from_redacted,
};
use crate::interactive::session::state::SessionState;
use saya_store::{FsSessionStore, SessionStore, StoredContinuation};
use saya_types::{CONTINUATION_VERSION, SessionContinuationV1, SourceBoundary, redact};

fn seeded_current(notes: Option<&str>) -> SessionContinuationV1 {
    let record = SessionContinuationV1 {
        version: CONTINUATION_VERSION,
        source: SourceBoundary {
            through_turn: 1,
            source_sha256: "b".repeat(64),
            workspace_root: None,
            profile: None,
        },
        narrative: None,
        user_notes: notes.map(str::to_owned),
        files: Vec::new(),
        checks: Vec::new(),
    };
    record.validate().expect("the seeded record validates");
    record
}

fn state_with_current(notes: Option<&str>) -> SessionState {
    let mut state = SessionState::new("s1", None, "m");
    state.continuation = Some(StoredContinuation::Current(seeded_current(notes)));
    state
}

fn current(state: &SessionState) -> &SessionContinuationV1 {
    match &state.continuation {
        Some(StoredContinuation::Current(record)) => record,
        other => panic!("expected a typed continuation, found {other:?}"),
    }
}

#[path = "notes_tests/formatting.rs"]
mod formatting;
#[path = "notes_tests/persistence.rs"]
mod persistence;
#[path = "notes_tests/refusals.rs"]
mod refusals;
#[path = "notes_tests/show.rs"]
mod show;
#[path = "notes_tests/updates.rs"]
mod updates;
