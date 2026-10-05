//! Tests for the compaction-to-continuation assembly: what a completed
//! compaction persists, what it refuses, and how the record round-trips
//! through the store and resume.

use super::*;
use crate::interactive::session_compact::{
    CompactionPlan, apply_with_continuation, plan as compact_plan,
};
use crate::interactive::session_continuation::record::FileObservation;
use crate::interactive::session_resume::{SessionDefaults, block_on, state_from_redacted};
use crate::interactive::session_state::SessionState;
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

/// A compaction that completes saves a typed continuation: the deterministic
/// facts carry the observed edit with the injected digest, the clean summary
/// becomes the generated narrative, the source boundary is the persisted turn
/// count, and the transcript (`messages`, `turns`) is unchanged.
#[test]
fn compaction_saves_a_continuation_with_facts_and_a_clean_narrative() {
    let mut state = turns(5);
    observe_edit(&mut state, "src/a.rs");
    let before_messages = state.messages.clone();
    let before_turns = state.turns.clone();
    let plan = plan_for(&state);
    apply_with_continuation(&mut state, &plan, SUMMARY, &digest_of)
        .expect("a valid summary applies");
    let record = current(&state);
    assert_eq!(
        record.source.through_turn, 5,
        "the boundary is the turn count"
    );
    assert_eq!(
        record.files.len(),
        1,
        "the observed edit is a fact: {record:?}"
    );
    assert_eq!(record.files[0].path, "src/a.rs");
    assert_eq!(
        record.files[0].sha256.as_deref(),
        Some(digest64().as_str()),
        "the digest comes from the injected resolver"
    );
    let narrative = record
        .narrative
        .as_ref()
        .expect("a clean narrative survives");
    assert_eq!(narrative.text, SUMMARY);
    assert_eq!(narrative.source, NarrativeSource::Summarizer);
    record.validate().expect("the record validates");
    assert_eq!(
        state.messages, before_messages,
        "the transcript is unchanged"
    );
    assert_eq!(state.turns, before_turns, "the turns are unchanged");
}

/// A summary the redactor would rewrite is OMITTED from the continuation —
/// the facts still save. The live working-memory summary is unchanged.
#[test]
fn a_sensitive_narrative_is_omitted_but_facts_survive() {
    let summary = "setup used password=hunter2 and finished the edit.";
    assert_ne!(
        redact(summary),
        summary,
        "the test needs a sensitive summary"
    );
    let mut state = turns(5);
    observe_edit(&mut state, "src/a.rs");
    let plan = plan_for(&state);
    apply_with_continuation(&mut state, &plan, summary, &digest_of).expect("applies");
    let record = current(&state);
    assert!(
        record.narrative.is_none(),
        "a sensitive narrative must be omitted: {:?}",
        record.narrative
    );
    assert_eq!(record.files.len(), 1, "the facts survive the omission");
    record.validate().expect("the fact-only record validates");
    assert_eq!(
        state.compaction_summary.as_deref(),
        Some(summary),
        "the live working memory keeps the summary as before"
    );
}

/// The model's prose mints no facts: a summary claiming edits and passing
/// tests records no file or check the tools did not observe.
#[test]
fn a_forged_summary_adds_no_facts() {
    let mut state = turns(5);
    let plan = plan_for(&state);
    apply_with_continuation(
        &mut state,
        &plan,
        "I edited secret.rs and all tests passed.",
        &digest_of,
    )
    .expect("applies");
    let record = current(&state);
    assert!(
        record.files.is_empty() && record.checks.is_empty(),
        "prose minted facts: {record:?}"
    );
}

/// User-authored notes ride across a new compaction, preserved apart from the
/// generated narrative — the new record carries them and a fresh boundary.
#[test]
fn existing_user_notes_are_preserved_across_a_new_compaction() {
    let mut state = turns(5);
    state.continuation = Some(StoredContinuation::Current(seeded_current(Some(
        "keep these notes",
    ))));
    let plan = plan_for(&state);
    apply_with_continuation(&mut state, &plan, SUMMARY, &digest_of).expect("applies");
    let record = current(&state);
    assert_eq!(
        record.user_notes.as_deref(),
        Some("keep these notes"),
        "the notes ride the new record"
    );
    assert_ne!(
        record.source.source_sha256,
        "b".repeat(64),
        "the boundary is the new compaction's, not the seeded one"
    );
    assert!(
        record.narrative.is_some(),
        "the narrative is still generated"
    );
}

/// An `Unsupported` continuation is a newer binary's data: a compaction
/// applies its working-memory change but never replaces the record.
#[test]
fn an_unsupported_continuation_is_never_replaced() {
    let mut state = turns(5);
    let unsupported = StoredContinuation::Unsupported(serde_json::json!({"version": 2}));
    state.continuation = Some(unsupported.clone());
    let plan = plan_for(&state);
    apply_with_continuation(&mut state, &plan, SUMMARY, &digest_of).expect("applies");
    assert_eq!(
        state.continuation,
        Some(unsupported),
        "the newer binary's continuation must survive the compaction"
    );
    assert!(
        state.compaction_summary.is_some(),
        "the working memory still compacts"
    );
}

/// An observation set that cannot validate refuses the whole assembly, and
/// the previous continuation stays untouched.
///
/// Deterministic shape, documented: the builder maps observations to facts
/// without re-filtering paths (the collector enforces the contract's path
/// rule at absorb time), so a fact whose path escapes the workspace — here a
/// hand-carried observation, the only way past a collector — fails
/// `validate()`, and `assemble_continuation` returns `None`.
#[test]
fn a_failed_validation_keeps_the_previous_continuation() {
    let mut state = turns(5);
    state.continuation = Some(StoredContinuation::Current(seeded_current(Some(
        "previous",
    ))));
    state.observations.files.push(FileObservation {
        path: "../escape.rs".into(),
        tool: "workspace_edit".into(),
        observed_unix_ms: NOW,
    });
    let plan = plan_for(&state);
    apply_with_continuation(&mut state, &plan, SUMMARY, &digest_of).expect("applies");
    let record = current(&state);
    assert_eq!(
        record.user_notes.as_deref(),
        Some("previous"),
        "the previous continuation is untouched"
    );
    assert_eq!(
        record.source.source_sha256,
        "b".repeat(64),
        "no new record replaced the seeded one"
    );
    assert!(
        record.files.is_empty(),
        "the invalid fact was not persisted"
    );
}

/// A typed continuation round-trips: `redacted()` hands it to the store, the
/// store persists it, and a resume carries the same record back.
#[test]
fn a_continuation_round_trips_through_redacted_and_resume() {
    let root = store_root("round-trip");
    let store = FsSessionStore::new(&root);
    let mut state = turns(2);
    observe_edit(&mut state, "src/a.rs");
    let record = assemble_continuation(&state, "carried summary.", &digest_of).expect("assembles");
    state.continuation = Some(StoredContinuation::Current(record));
    block_on(store.save(state.redacted())).expect("the record saves");
    let loaded = block_on(store.load("s1"))
        .expect("loads")
        .expect("the record is there");
    let resumed = state_from_redacted(loaded, &defaults());
    assert_eq!(
        resumed.continuation, state.continuation,
        "the typed continuation round-trips through the store and resume"
    );
    assert_eq!(resumed.parent, state.parent);
    let _ = std::fs::remove_dir_all(root);
}

/// A continuation this binary does not understand loads as `Unsupported`,
/// survives a resume, and a save carrying it verbatim succeeds while a save
/// that would drop it is refused — the old `redacted()` (hard-coding
/// `continuation: None`) would have made every later save of the session
/// fail.
#[test]
fn an_unsupported_continuation_survives_resume_and_save() {
    let root = store_root("unsupported");
    let store = FsSessionStore::new(&root);
    let state = turns(2);
    block_on(store.save(state.redacted())).expect("the base record saves");
    let path = root.join("s1.json");
    let mut value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("parses");
    value["continuation"] = serde_json::json!({"version": 2, "future": true});
    std::fs::write(&path, serde_json::to_string_pretty(&value).expect("writes")).expect("write");

    let loaded = block_on(store.load("s1")).expect("loads").expect("present");
    let resumed = state_from_redacted(loaded, &defaults());
    let Some(StoredContinuation::Unsupported(raw)) = &resumed.continuation else {
        panic!(
            "an unknown continuation must load as Unsupported: {:?}",
            resumed.continuation
        );
    };
    assert_eq!(raw, &serde_json::json!({"version": 2, "future": true}));

    block_on(store.save(resumed.redacted()))
        .expect("a save carrying the Unsupported record verbatim succeeds");
    let file_value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("parses");
    assert_eq!(
        file_value["continuation"],
        serde_json::json!({"version": 2, "future": true}),
        "the unsupported record is preserved verbatim: {file_value}"
    );

    let mut dropped = resumed.redacted();
    dropped.continuation = None;
    let error = block_on(store.save(dropped))
        .expect_err("dropping an unsupported continuation must be refused");
    assert_eq!(error, StoreError::UnsupportedContinuation);
    let _ = std::fs::remove_dir_all(root);
}

/// The parent lineage link rides the record the same way and round-trips.
#[test]
fn the_parent_link_round_trips() {
    let root = store_root("parent");
    let store = FsSessionStore::new(&root);
    let mut state = turns(2);
    state.parent = Some(ParentLink {
        id: "parent-1".into(),
        rotation_id: "rot-1".into(),
    });
    block_on(store.save(state.redacted())).expect("saves");
    let loaded = block_on(store.load("s1")).expect("loads").expect("present");
    let resumed = state_from_redacted(loaded, &defaults());
    assert_eq!(
        resumed.parent, state.parent,
        "the lineage link round-trips through the store and resume"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// A failed save leaves the last valid record on disk intact, and nothing in
/// memory claims the new continuation is durable: the state keeps it for a
/// retry, and once the oversize turn is removed the retry persists it.
#[test]
fn a_failed_save_keeps_the_previous_valid_record() {
    let root = store_root("failed-save");
    let store = FsSessionStore::new(&root);
    let mut state = turns(2);
    let first = assemble_continuation(&state, "first summary.", &digest_of).expect("assembles");
    state.continuation = Some(StoredContinuation::Current(first.clone()));
    block_on(store.save(state.redacted())).expect("the first record saves");

    let second = assemble_continuation(&state, "second summary.", &digest_of).expect("assembles");
    state.continuation = Some(StoredContinuation::Current(second.clone()));

    state.record_turn("x".repeat(MAX_SESSION_BYTES), "answer", false, Vec::new());
    let error =
        block_on(store.save(state.redacted())).expect_err("an oversize record must be refused");
    assert_eq!(error, StoreError::LimitExceeded);
    let loaded = block_on(store.load("s1")).expect("loads").expect("present");
    assert_eq!(
        loaded.continuation,
        Some(StoredContinuation::Current(first)),
        "the last valid record kept its continuation"
    );
    assert_eq!(
        state.continuation,
        Some(StoredContinuation::Current(second.clone())),
        "the state keeps the new continuation for a retry; no flag claims it durable"
    );

    state.turns.pop();
    state.messages.pop();
    state.messages.pop();
    block_on(store.save(state.redacted())).expect("the retry saves once the oversize turn is gone");
    let loaded = block_on(store.load("s1")).expect("loads").expect("present");
    assert_eq!(
        loaded.continuation,
        Some(StoredContinuation::Current(second)),
        "the retry persists the new continuation"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// `/clear` drops the continuation it understands (it described the cleared
/// conversation), keeps a newer binary's `Unsupported` record, and keeps the
/// parent lineage link.
#[test]
fn clear_drops_a_current_continuation_but_keeps_an_unsupported_one_and_the_parent() {
    let mut state = turns(5);
    let plan = plan_for(&state);
    apply_with_continuation(&mut state, &plan, SUMMARY, &digest_of).expect("applies");
    state.parent = Some(ParentLink {
        id: "parent-1".into(),
        rotation_id: "rot-1".into(),
    });
    state.apply(SlashCommand::Clear, &[]);
    assert!(
        state.continuation.is_none(),
        "/clear must drop the typed continuation: {:?}",
        state.continuation
    );
    assert_eq!(
        state.parent.as_ref().map(|link| link.id.as_str()),
        Some("parent-1"),
        "the lineage link survives /clear"
    );

    let mut state = turns(5);
    let unsupported = StoredContinuation::Unsupported(serde_json::json!({"version": 2}));
    state.continuation = Some(unsupported.clone());
    state.apply(SlashCommand::Clear, &[]);
    assert_eq!(
        state.continuation,
        Some(unsupported),
        "a newer binary's continuation must survive /clear"
    );
}
