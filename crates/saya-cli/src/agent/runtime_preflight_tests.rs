//! Production preflight at the real runtime boundary.
//!
//! A too-small known `context_window_tokens` refuses before any provider
//! request; a generous one dispatches. Uses the real production entry
//! (`run_prompt_with_sink_and_outcomes`, which builds the live HTTP provider
//! from config) against a local counting source and a real SQLite connector,
//! so the real tool definitions/schemas are in the request.

use crate::agent::runtime::{PromptOverrides, SinkPrompt, run_prompt_with_sink_and_outcomes};
use async_trait::async_trait;
use saya_agent::{AgentEvent, AgentEventSink, AgentMode, ApprovalPolicy, BudgetDimension};
use std::sync::{Arc, Mutex, mpsc};

#[path = "runtime_preflight_tests/http_source.rs"]
mod http_source;
#[path = "runtime_preflight_tests/runtime_fixture.rs"]
mod runtime_fixture;
#[path = "runtime_preflight_second_tests.rs"]
mod second_tests;

pub(super) use http_source::counting_source;
pub(super) use runtime_fixture::{runtime_for_source, temp_root};

pub(super) struct RecordingSink {
    pub(super) events: Arc<Mutex<Vec<AgentEvent>>>,
}

#[async_trait]
impl AgentEventSink for RecordingSink {
    async fn emit(&self, event: AgentEvent) {
        self.events.lock().expect("event log lock").push(event);
    }
}

pub(super) const ANSWER_RESPONSE: &str = "data: {\"choices\":[{\"delta\":{\"content\":\"done.\"},\"finish_reason\":\"stop\"}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":1}}\n\ndata: [DONE]\n\n";

#[tokio::test]
async fn a_too_small_known_window_refuses_before_any_provider_request() {
    let root = temp_root("refuse");
    let seen = Arc::new(Mutex::new(0_usize));
    let (release_tx, release_rx) = mpsc::channel();
    let (base_url, source) = counting_source(Arc::clone(&seen), release_rx);
    let runtime = runtime_for_source(&root, base_url, Some(64)).await;
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: Arc::clone(&events),
    };
    let cancellation = saya_agent::CancellationToken::new();
    let output = run_prompt_with_sink_and_outcomes(SinkPrompt {
        runtime: &runtime,
        prompt: "count rows",
        approval: ApprovalPolicy::ReadOnly,
        can_prompt: false,
        can_obtain_approval: false,
        overrides: PromptOverrides::default(),
        history: Vec::new(),
        sink: &sink,
        cancellation: cancellation.clone(),
        question_scope: None,
        state_db: None,
        decider: None,
        last_sql: None,
        session: None,
        agent_mode: AgentMode::Build,
        capture: None,
        prior_tool_outcomes: None,
        continuation: None,
        compaction_summary: None,
    })
    .await
    .expect("a preflight refusal is truncated output, not an error");
    release_tx.send(()).expect("release source");
    source.join().expect("source thread joins");
    assert_eq!(
        *seen.lock().expect("request count lock"),
        0,
        "a too-small known window refuses before any provider request"
    );
    assert!(
        output.truncated,
        "the refusal is truncated output: {output:?}"
    );
    let stops: Vec<BudgetDimension> = events
        .lock()
        .expect("event log lock")
        .iter()
        .filter_map(|event| match event {
            AgentEvent::AnsweringBudgetStopped { dimension } => Some(*dimension),
            _ => None,
        })
        .collect();
    assert_eq!(
        stops,
        vec![BudgetDimension::RequestContextWindow],
        "exactly one RequestContextWindow stop: {stops:?}"
    );
    assert!(
        !output.used_bounded_sql_query,
        "no tool/SQL effect happened: {output:?}"
    );
    assert_eq!(
        cancellation.reason(),
        Some(saya_agent::CancellationReason::Budget(
            BudgetDimension::RequestContextWindow
        )),
        "the refusal latches the typed budget reason"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn a_generous_window_dispatches_normally() {
    second_tests::generous_dispatch().await;
}

#[tokio::test]
async fn the_shared_question_scope_survives_a_preflight_refusal() {
    second_tests::scope_survives_refusal().await;
}
