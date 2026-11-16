//! One real turn records its metrics into the question scope through the
//! wrapped sink, reusing the preflight fixture's local provider and runtime.

use super::preflight_tests::{RecordingSink, counting_source, runtime_for_source, temp_root};
use crate::agent::question_scope::QuestionScope;
use crate::agent::runtime::{PromptOverrides, SinkPrompt, run_prompt_with_sink_and_outcomes};
use crate::local_metrics::{LocalMetrics, LocalPhaseSnapshot, phase_index};
use crate::metrics_sink::MetricsSink;
use saya_agent::{AgentEvent, AgentMetricPhase, AgentMode, ApprovalPolicy, CancellationToken};
use std::sync::{Arc, Mutex, mpsc};

#[tokio::test]
async fn a_real_turn_records_into_the_question_scope_through_the_wrapped_sink() {
    let root = temp_root("metrics");
    let seen = Arc::new(Mutex::new(0_usize));
    let (release_tx, release_rx) = mpsc::channel();
    let (base_url, source) = counting_source(Arc::clone(&seen), release_rx);
    let runtime = runtime_for_source(&root, base_url, Some(1_000_000)).await;
    let cancellation = CancellationToken::new();
    let scope = QuestionScope::with_metrics(
        &runtime,
        cancellation.clone(),
        Some(Arc::new(LocalMetrics::new())),
    );
    let events = Arc::new(Mutex::new(Vec::new()));
    let recording = RecordingSink {
        events: Arc::clone(&events),
    };
    let wrapped = MetricsSink::new(&recording, scope.metrics());
    let output = run_prompt_with_sink_and_outcomes(SinkPrompt {
        runtime: &runtime,
        prompt: "count rows",
        approval: ApprovalPolicy::ReadOnly,
        can_prompt: false,
        can_obtain_approval: false,
        overrides: PromptOverrides::default(),
        history: Vec::new(),
        sink: &wrapped,
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
    .expect("one natural turn completes");
    release_tx.send(()).expect("release source");
    source.join().expect("source thread joins");
    assert!(!output.truncated, "the turn completes: {output:?}");
    let snapshot = scope
        .metrics()
        .expect("this question carries metrics")
        .snapshot();
    let establishment = &snapshot.phases
        [phase_index(AgentMetricPhase::ProviderEstablishment).expect("known phase indexed")];
    assert!(
        establishment.samples >= 1,
        "provider establishment recorded: {snapshot:?}"
    );
    let attempt =
        &snapshot.phases[phase_index(AgentMetricPhase::LocalAttempt).expect("known phase indexed")];
    assert!(attempt.samples >= 1, "local attempt recorded: {snapshot:?}");
    assert!(
        snapshot.first_useful.is_some(),
        "first useful recorded: {snapshot:?}"
    );
    let recorded = events.lock().expect("event log lock");
    assert!(
        !recorded.is_empty(),
        "events forwarded through the wrapper: {recorded:?}"
    );
    assert!(
        recorded.iter().any(
            |event| matches!(event, AgentEvent::AssistantText { text } if text.contains("done."))
        ),
        "the answer text flowed through the wrapper: {recorded:?}"
    );
    drop(recorded);
    let other = QuestionScope::with_metrics(
        &runtime,
        CancellationToken::new(),
        Some(Arc::new(LocalMetrics::new())),
    );
    assert_eq!(
        other.metrics().expect("enabled question").snapshot(),
        LocalPhaseSnapshot::default(),
        "a second question starts with an empty snapshot"
    );
    let _ = std::fs::remove_dir_all(root);
}
