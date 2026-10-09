//! The pure rotation successor: what a compacted parent hands its successor,
//! what it refuses, and that the successor is strictly smaller than its
//! parent — with the round trip through the real store and resume.

use super::{RotateRefusal, rotate_state};
use crate::interactive::session::continuation::PriorToolOutcomes;
use crate::interactive::session::continuation::compaction::manual::{
    apply_with_continuation, plan as compact_plan,
};
use crate::interactive::session::continuation::source_digest;
use crate::interactive::session::lifecycle::resume::{
    SessionDefaults, block_on, state_from_redacted,
};
use crate::interactive::session::state::SessionState;
use crate::interactive::tui::types::SessionUsage;
use saya_agent::{AgentOutput, TokenUsage, ToolMetadata};
use saya_store::{FsSessionStore, ParentLink, SessionStore, StoredContinuation};
use saya_types::{
    CONTINUATION_VERSION, CheckFact, CheckOutcome, FactProvenance, GeneratedNarrative,
    NarrativeSource, SessionContinuationV1, SessionTask, SessionTaskList, SourceBoundary,
    TaskStatus,
};

const NOW: i64 = 1_700_000_000_000;
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

/// One completed write-shaped tool call: the observation the continuation
/// facts are built from.
fn completed_write(path: &str) -> AgentOutput {
    AgentOutput {
        answer: "done".into(),
        events: Vec::new(),
        used_bounded_sql_query: false,
        tool_metadata: vec![ToolMetadata {
            name: "workspace_write".into(),
            status: "completed".into(),
            arguments: format!(r#"{{"path":"{path}"}}"#),
            result_shape: None,
        }],
        usage: TokenUsage::default(),
        learning_usage: None,
        truncated: false,
        answer_sql: None,
    }
}

fn store_root(label: &str) -> std::path::PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-rotate-{label}-{}-{stamp}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create the store root");
    root
}

/// A compacted parent with everything a successor may carry — and everything
/// it must NOT carry: notes, file and check facts via the record, live
/// observations, prior outcomes, usage, and a denial.
fn compacted_parent() -> SessionState {
    let mut state = turns(6);
    state.profile = Some("analytics".into());
    state.workspace_root = Some("/projects/saya".into());
    state.approval_mode = "always".into();
    state.agent_mode = "plan".into();
    state.allow_data_sharing = true;
    state.included_profiles = vec!["archive".into()];
    state.denied_programs = vec!["curl".into()];
    state.task_list = SessionTaskList::new(vec![
        SessionTask::new("ship the release", TaskStatus::Pending).expect("a valid task"),
    ])
    .expect("a valid task list");
    state
        .observations
        .absorb(&completed_write("SESSION-NOTES.md"), NOW);
    state.prior_tool_outcomes = Some(PriorToolOutcomes::from_live_output(&completed_write("x")));
    state.usage.record(&TokenUsage::new(100, 50));
    compact(&mut state, SUMMARY);
    let Some(StoredContinuation::Current(mut record)) = state.continuation.clone() else {
        panic!("the compaction saved a typed record");
    };
    record.user_notes = Some("keep the sqlite copy at backups/main.db".into());
    record.checks = vec![CheckFact {
        label: "cargo test".into(),
        outcome: CheckOutcome::Passed,
        observed_unix_ms: NOW,
        provenance: FactProvenance::ToolObserved {
            tool: "run_command".into(),
        },
    }];
    record
        .validate()
        .expect("the extended record passes the contract");
    state.continuation = Some(StoredContinuation::Current(record));
    state
}

fn current_record(state: &SessionState) -> SessionContinuationV1 {
    match &state.continuation {
        Some(StoredContinuation::Current(record)) => record.clone(),
        other => panic!("expected a typed record: {other:?}"),
    }
}
#[path = "session_rotate_tests/refusals.rs"]
mod refusals;
#[path = "session_rotate_tests/sharing.rs"]
mod sharing;
#[path = "session_rotate_tests/successor.rs"]
mod successor;
