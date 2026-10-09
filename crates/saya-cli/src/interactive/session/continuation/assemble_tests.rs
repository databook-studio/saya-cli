//! Tests for the compaction-to-continuation assembly: what a completed
//! compaction persists, what it refuses, and how the record round-trips
//! through the store and resume.

use super::*;
use crate::interactive::session::continuation::compaction::manual::{
    CompactionPlan, apply_with_continuation, plan as compact_plan,
};
use crate::interactive::session::continuation::record::FileObservation;
use crate::interactive::session::lifecycle::resume::{
    SessionDefaults, block_on, state_from_redacted,
};
use crate::interactive::session::state::SessionState;
use crate::slash::SlashCommand;
use saya_agent::{AgentOutput, TokenUsage, ToolMetadata};
use saya_store::{
    FsSessionStore, MAX_SESSION_BYTES, ParentLink, SessionStore, StoreError, StoredContinuation,
};
use saya_types::{CONTINUATION_VERSION, NarrativeSource, SourceBoundary, redact};

const NOW: i64 = 1_700_000_000_000;
const SUMMARY: &str = "older turns discussed the edit.";

fn turns(count: usize) -> SessionState {
    let mut state = SessionState::new("s1", None, "m");
    for index in 0..count {
        state.record_turn(format!("q{index}"), format!("a{index}"), false, Vec::new());
    }
    state
}

fn write_tool(path: &str) -> ToolMetadata {
    ToolMetadata {
        name: "workspace_edit".into(),
        status: "completed".into(),
        arguments: format!(r#"{{"path":"{path}"}}"#),
        result_shape: None,
    }
}

fn output(answer: &str, tools: Vec<ToolMetadata>) -> AgentOutput {
    AgentOutput {
        answer: answer.into(),
        events: Vec::new(),
        used_bounded_sql_query: false,
        tool_metadata: tools,
        usage: TokenUsage::default(),
        learning_usage: None,
        truncated: false,
        answer_sql: None,
    }
}

fn digest64() -> String {
    "a".repeat(64)
}

/// The injected resolver: only `src/a.rs` resolves, to a fixed digest.
fn digest_of(path: &str) -> Option<String> {
    (path == "src/a.rs").then(digest64)
}

fn observe_edit(state: &mut SessionState, path: &str) {
    state
        .observations
        .absorb(&output("done", vec![write_tool(path)]), NOW);
}

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

fn current(state: &SessionState) -> &SessionContinuationV1 {
    match &state.continuation {
        Some(StoredContinuation::Current(record)) => record,
        other => panic!("expected a typed continuation, found {other:?}"),
    }
}

fn plan_for(state: &SessionState) -> CompactionPlan {
    compact_plan(&state.turns, &state.provider_history()).expect("a long history plans")
}

fn defaults() -> SessionDefaults {
    SessionDefaults {
        provider: "ollama".into(),
        model: "m".into(),
        allow_data_sharing: false,
        approval_mode: "ask".into(),
    }
}

fn store_root(label: &str) -> std::path::PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-continuation-{label}-{}-{stamp}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create the store root");
    root
}

include!("assemble_tests/assembly_facts.rs");

include!("assemble_tests/validation_and_resume.rs");

include!("assemble_tests/persistence.rs");
