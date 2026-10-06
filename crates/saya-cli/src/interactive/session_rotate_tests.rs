//! The pure rotation successor: what a compacted parent hands its successor,
//! what it refuses, and that the successor is strictly smaller than its
//! parent — with the round trip through the real store and resume.

use super::{RotateRefusal, rotate_state};
use crate::interactive::session_compact::{apply_with_continuation, plan as compact_plan};
use crate::interactive::session_continuation::PriorToolOutcomes;
use crate::interactive::session_continuation::source_digest;
use crate::interactive::session_resume::{SessionDefaults, block_on, state_from_redacted};
use crate::interactive::session_state::SessionState;
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

/// A compacted parent rotates into a successor that carries the summary,
/// notes, and facts plus the not-yet-summarised tail — and nothing else: no
/// usage, observations, denials, or prior outcomes ride across, and the
/// parent is never mutated.
#[test]
fn a_compacted_parent_rotates_into_a_small_successor() {
    let parent = compacted_parent();
    let snapshot = parent.clone();
    let record = current_record(&parent);
    assert_eq!(
        parent.compacted_turns, 4,
        "fixture sanity: 4 of 6 compacted"
    );
    let successor = rotate_state(&parent, "s2", "rot-1").expect("a compacted parent rotates");
    assert_eq!(parent, snapshot, "the parent is never mutated");

    assert_eq!(successor.id, "s2");
    assert_eq!(
        successor.parent,
        Some(ParentLink {
            id: "s1".into(),
            rotation_id: "rot-1".into(),
        }),
        "the link is lineage only"
    );
    assert_eq!(
        successor.turns,
        parent.turns[parent.compacted_turns..].to_vec(),
        "the verbatim not-yet-summarised tail rides across"
    );
    assert_eq!(successor.turns.len(), 2, "fixture sanity: the 2 kept turns");
    assert_eq!(successor.turns[0].user, "q4");
    assert_eq!(successor.compaction_summary.as_deref(), Some(SUMMARY));
    assert_eq!(successor.compacted_turns, 0);
    assert!(
        successor.messages.is_empty(),
        "the transcript does not ride"
    );
    assert!(
        successor.prior_tool_outcomes.is_none(),
        "a successor inherits no live outcome status"
    );
    assert_eq!(
        successor.observations,
        Default::default(),
        "a successor inherits no observations"
    );
    assert!(
        successor.denied_programs.is_empty(),
        "a successor inherits NO denials"
    );
    assert_eq!(
        successor.usage,
        SessionUsage::default(),
        "a successor starts with a fresh token total"
    );
    assert!(!successor.context_warned);
    assert!(!successor.auto_compact_failed);

    let Some(StoredContinuation::Current(carried)) = successor.continuation.as_ref() else {
        panic!("the successor carries a typed record");
    };
    carried
        .validate()
        .expect("the carried record passes the contract");
    assert_eq!(
        carried.source.through_turn, 0,
        "it summarises none of these turns"
    );
    assert_eq!(
        carried.source.source_sha256,
        source_digest(&[]),
        "the boundary digests the empty prefix"
    );
    assert_eq!(
        carried.source.workspace_root.as_deref(),
        Some("/projects/saya"),
        "the boundary carries the workspace"
    );
    assert_eq!(
        carried.source.profile.as_deref(),
        Some("analytics"),
        "the boundary carries the profile"
    );
    assert_eq!(
        carried.narrative, record.narrative,
        "the narrative is carried"
    );
    assert_eq!(carried.user_notes, record.user_notes, "notes ride across");
    assert_eq!(carried.files, record.files, "file facts ride across");
    assert_eq!(carried.checks, record.checks, "check facts ride across");

    assert_eq!(successor.profile.as_deref(), Some("analytics"));
    assert_eq!(successor.workspace_root.as_deref(), Some("/projects/saya"));
    assert_eq!(successor.approval_mode, parent.approval_mode);
    assert_eq!(successor.agent_mode, parent.agent_mode);
    assert_eq!(successor.provider, parent.provider);
    assert_eq!(successor.model, parent.model);
    assert_eq!(successor.allow_data_sharing, parent.allow_data_sharing);
    assert_eq!(successor.included_profiles, parent.included_profiles);
    assert_eq!(successor.task_list, parent.task_list);
    assert_eq!(successor.provider_endpoint, parent.provider_endpoint);
    assert_eq!(
        successor.provider_endpoint_bound,
        parent.provider_endpoint_bound
    );
}

/// A successor built from a large parent serializes strictly smaller than
/// the parent: the whole point of rotation is the small record.
#[test]
fn a_successor_from_a_large_parent_is_strictly_smaller_than_its_parent() {
    let root = store_root("size");
    let store = FsSessionStore::new(&root);
    let mut state = turns(200);
    compact(&mut state, SUMMARY);
    let parent_bytes = store
        .capacity_of(state.redacted())
        .expect("the parent measures")
        .used_bytes;
    let successor = rotate_state(&state, "s2", "rot-big").expect("a compacted parent rotates");
    let successor_bytes = store
        .capacity_of(successor.redacted())
        .expect("the successor measures")
        .used_bytes;
    assert!(
        successor_bytes < parent_bytes,
        "the successor ({successor_bytes}B) must be smaller than the parent ({parent_bytes}B)"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// The published successor saves through the real store and its carried
/// summary is restored on resume: summary present, `compacted_turns` zero,
/// lineage persisted.
#[test]
fn a_published_successor_resumes_with_its_carried_summary() {
    let root = store_root("resume");
    let parent = compacted_parent();
    let successor = rotate_state(&parent, "s2", "rot-1").expect("a compacted parent rotates");
    let store = FsSessionStore::new(&root);
    block_on(store.save(successor.redacted())).expect("the successor saves");
    let loaded = block_on(store.load("s2"))
        .expect("loads")
        .expect("the record is there");
    let resumed = state_from_redacted(loaded, &defaults());
    assert_eq!(
        resumed.compaction_summary.as_deref(),
        Some(SUMMARY),
        "the carried narrative is restored as the live summary"
    );
    assert_eq!(resumed.compacted_turns, 0);
    assert_eq!(
        resumed.parent,
        Some(ParentLink {
            id: "s1".into(),
            rotation_id: "rot-1".into(),
        }),
        "the lineage link persists"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// No continuation at all: nothing compacted, nothing to carry.
#[test]
fn an_uncompacted_session_refuses_rotation_as_nothing_compacted() {
    let state = turns(6);
    assert!(matches!(
        rotate_state(&state, "s2", "rot-1"),
        Err(RotateRefusal::NothingCompacted)
    ));
}

/// A newer binary's continuation is never interpreted, so it cannot be
/// rotated either.
#[test]
fn an_unsupported_continuation_refuses_rotation_as_unsupported() {
    let mut state = turns(6);
    state.continuation = Some(StoredContinuation::Unsupported(serde_json::json!({
        "version": 2, "future": true
    })));
    assert!(matches!(
        rotate_state(&state, "s2", "rot-1"),
        Err(RotateRefusal::Unsupported)
    ));
}

/// The record's narrative was withheld at save time: a successor would start
/// without context, so rotation refuses.
#[test]
fn a_narrative_less_record_refuses_rotation_as_no_narrative() {
    let mut state = compacted_parent();
    if let Some(StoredContinuation::Current(record)) = state.continuation.as_mut() {
        record.narrative = None;
    }
    assert!(matches!(
        rotate_state(&state, "s2", "rot-1"),
        Err(RotateRefusal::NoNarrative)
    ));
}

/// A valid typed record with a narrative on a session whose live state is
/// not compacted — e.g. just resumed with the record rejected — has nothing
/// to slice: refusal, not a whole-transcript successor.
#[test]
fn a_not_actually_compacted_session_refuses_rotation_as_nothing_compacted() {
    let mut state = turns(6);
    let record = SessionContinuationV1 {
        version: CONTINUATION_VERSION,
        source: SourceBoundary {
            through_turn: 6,
            source_sha256: source_digest(&state.turns),
            workspace_root: None,
            profile: None,
        },
        narrative: Some(GeneratedNarrative {
            text: "a carried-looking summary".into(),
            source: NarrativeSource::Summarizer,
        }),
        user_notes: None,
        files: Vec::new(),
        checks: Vec::new(),
    };
    record
        .validate()
        .expect("the hand-built record passes the contract");
    state.continuation = Some(StoredContinuation::Current(record));
    assert!(matches!(
        rotate_state(&state, "s2", "rot-1"),
        Err(RotateRefusal::NothingCompacted)
    ));
}

/// A record that fails its own contract cannot seed a successor.
#[test]
fn an_invalid_record_refuses_rotation_as_invalid_continuation() {
    let mut state = compacted_parent();
    if let Some(StoredContinuation::Current(record)) = state.continuation.as_mut() {
        record.source.source_sha256 = "nope".into();
    }
    assert!(matches!(
        rotate_state(&state, "s2", "rot-1"),
        Err(RotateRefusal::InvalidContinuation)
    ));
}

/// Every refusal names its exact user-facing sentence.
#[test]
fn every_refusal_names_its_sentence() {
    assert_eq!(
        RotateRefusal::NothingCompacted.sentence(),
        "Nothing to carry over yet: run /compact first so the session has a summary."
    );
    assert_eq!(
        RotateRefusal::NoNarrative.sentence(),
        "The saved summary was withheld (it looked sensitive), so a successor would \
         start without context. Rotation was not done."
    );
    assert_eq!(
        RotateRefusal::Unsupported.sentence(),
        "This session was written by a newer version; it cannot be rotated here."
    );
    assert_eq!(
        RotateRefusal::InvalidContinuation.sentence(),
        "The saved summary no longer validates; run /compact again, then rotate."
    );
}

/// A successor's carried summary (`compacted_turns == 0`) replays under a
/// local provider: it reaches `provider_history()[0]` with the carried-over
/// header, since its source turns cannot be re-checked here.
#[test]
fn a_successor_replays_its_carried_summary_under_a_local_provider() {
    let parent = compacted_parent();
    let successor = rotate_state(&parent, "s2", "rot-1").expect("a compacted parent rotates");
    assert_eq!(successor.compacted_turns, 0);
    let history = successor.provider_history();
    let first = history.first().expect("the carried summary leads");
    assert_eq!(first.role, "user");
    assert!(first.content.contains(SUMMARY));
    assert!(
        first
            .content
            .contains("carried over from the previous session")
    );
}

/// The same successor under a cloud provider with sharing refused withholds
/// the carried summary everywhere: it is potentially database-derived and
/// its source turns cannot be re-checked.
#[test]
fn a_successor_withholds_its_carried_summary_when_sharing_is_refused() {
    let parent = compacted_parent();
    let mut successor = rotate_state(&parent, "s2", "rot-1").expect("a compacted parent rotates");
    successor.provider = "openai".into();
    successor.allow_data_sharing = false;
    assert!(successor.provider_endpoint.is_none());
    assert!(
        successor.compaction_summary_text().is_none(),
        "the raw summary is gated off"
    );
    let history = successor.provider_history();
    assert!(
        history
            .iter()
            .all(|message| !message.content.contains(SUMMARY)),
        "no summary entry rides into the request"
    );
    assert_eq!(
        history.first().map(|message| message.content.as_str()),
        Some("q4"),
        "the verbatim tail still leads the request"
    );
}

/// Explicit sharing on the same cloud successor admits the carried summary.
#[test]
fn a_successor_replays_its_carried_summary_when_sharing_is_allowed() {
    let parent = compacted_parent();
    let mut successor = rotate_state(&parent, "s2", "rot-1").expect("a compacted parent rotates");
    successor.provider = "openai".into();
    successor.allow_data_sharing = true;
    let history = successor.provider_history();
    let first = history.first().expect("the carried summary leads");
    assert!(first.content.contains(SUMMARY));
    assert!(
        first
            .content
            .contains("carried over from the previous session")
    );
}

/// A normally compacted state (`compacted_turns > 0`) keeps its original
/// gate behavior in both privacy postures: withheld only when a
/// database-derived turn exists under refused sharing.
#[test]
fn a_normally_compacted_state_keeps_its_original_gate_behavior() {
    let mut local = turns(6);
    compact(&mut local, "kept summary");
    let compacted_turns = local.compacted_turns;
    assert!(compacted_turns > 0, "fixture sanity: a real compaction ran");
    local.provider = "ollama".into();
    local.allow_data_sharing = false;
    local.provider_endpoint = None;
    assert!(local.compaction_summary_text().is_some());
    assert!(
        local
            .compaction_narrative()
            .unwrap()
            .contains(&format!("compacted from {compacted_turns} turns"))
    );

    let mut cloud = turns(6);
    compact(&mut cloud, "kept summary");
    cloud.provider = "openai".into();
    cloud.allow_data_sharing = false;
    cloud.provider_endpoint = None;
    cloud.record_turn("q3", "row answer", true, Vec::new());
    assert!(cloud.compaction_summary_text().is_none());
    assert!(cloud.compaction_narrative().is_none());

    cloud.allow_data_sharing = true;
    assert!(cloud.compaction_summary_text().is_some());
}
