//! Tests 2-3: generous dispatch and scope survival (sibling split).

use super::*;
use crate::agent::runtime::QuestionScope;
use std::sync::{Arc, Mutex, mpsc};

pub(super) async fn generous_dispatch() {
    let root = temp_root("dispatch");
    let seen = Arc::new(Mutex::new(0_usize));
    let (release_tx, release_rx) = mpsc::channel();
    let (base_url, source) = counting_source(Arc::clone(&seen), release_rx);
    let runtime = runtime_for_source(&root, base_url, Some(1_000_000)).await;
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: Arc::clone(&events),
    };
    let output = run_prompt_with_sink_and_outcomes(SinkPrompt {
        runtime: &runtime,
        prompt: "count rows",
        approval: ApprovalPolicy::ReadOnly,
        can_prompt: false,
        can_obtain_approval: false,
        overrides: PromptOverrides::default(),
        history: Vec::new(),
        sink: &sink,
        cancellation: saya_agent::CancellationToken::new(),
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
    .expect("a generous window dispatches");
    release_tx.send(()).expect("release source");
    source.join().expect("source thread joins");
    assert_eq!(
        *seen.lock().expect("request count lock"),
        1,
        "a generous window dispatches exactly one provider request"
    );
    assert!(!output.truncated, "the run completes naturally: {output:?}");
    assert!(
        output.answer.contains("done."),
        "mock answer text: {output:?}"
    );
    let _ = std::fs::remove_dir_all(root);
}

pub(super) async fn scope_survives_refusal() {
    let root = temp_root("scope");
    let seen = Arc::new(Mutex::new(0_usize));
    let (release_tx, release_rx) = mpsc::channel();
    let (base_url, source) = counting_source(Arc::clone(&seen), release_rx);
    let runtime = runtime_for_source(&root, base_url, Some(64)).await;
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: Arc::clone(&events),
    };
    let cancellation = saya_agent::CancellationToken::new();
    let scope = QuestionScope::new(&runtime, cancellation.clone());
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
        question_scope: Some(scope.clone()),
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
    .expect("a preflight refusal is a truncated output, not an error");
    release_tx.send(()).expect("release source");
    source.join().expect("source thread joins");
    assert!(output.truncated, "the refusal is truncated: {output:?}");
    assert_eq!(
        cancellation.reason(),
        Some(saya_agent::CancellationReason::Budget(
            BudgetDimension::RequestContextWindow
        )),
        "the token's first reason is the preflight budget"
    );
    let remaining = scope.begin_segment();
    scope.finish_segment();
    let _ = remaining;
    assert_eq!(
        *seen.lock().expect("request count lock"),
        0,
        "the refusing run sent no provider request"
    );
    let _ = std::fs::remove_dir_all(root);
}
