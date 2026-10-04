use super::*;
use saya_agent::{
    AllowReadOnlyApproval, NoopEventSink, run_agent_with_control, run_agent_with_sink,
};

#[tokio::test]
async fn required_prefix_refuses_known_window_before_admission_and_dispatches_when_roomy() {
    let req = request("é".repeat(250), Some("required system ".repeat(25)));
    let defs = vec![wide_definition("query", 320)];
    let limits = AgentLimits {
        context_byte_budget: 8 * 1024,
        ..AgentLimits::default()
    };
    let cancellation = CancellationToken::new();
    let provider = ScriptedProvider::new(vec![turn("unexpected dispatch", Vec::new())]).0;
    let executor = RecordingExecutor::default();
    let approval = RecordingApproval::default();
    let sink = RecordingSink::default();
    let refused = run(
        &provider,
        &executor,
        req.clone(),
        defs.clone(),
        control(
            limits,
            &approval,
            &sink,
            cancellation,
            preflight(Some(1024 + 64 + 900), 64),
        ),
    )
    .await
    .unwrap();
    assert!(stopped(&refused, BudgetDimension::RequestContextWindow));
    assert!(refused.truncated);
    assert_eq!(provider.requests.lock().unwrap().len(), 0);
    assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
    assert_eq!(approval.0.load(Ordering::SeqCst), 0);
    assert_eq!(
        *sink.observations.lock().unwrap(),
        [AnsweringObservation::Initialized],
        "refusal before provider dispatch must not open a request observation"
    );

    let (provider, requests) = ScriptedProvider::new(vec![turn("answer", Vec::new())]);
    let output = run(
        &provider,
        &executor,
        req,
        defs,
        control(
            limits,
            &approval,
            &RecordingSink::default(),
            CancellationToken::new(),
            preflight(Some(1024 + 64 + 5000), 64),
        ),
    )
    .await
    .unwrap();
    assert_eq!(output.answer, "answer");
    assert_eq!(requests.lock().unwrap().len(), 1);
    assert!(provider_projection_bytes(&requests.lock().unwrap()[0]) <= 5000);
}

#[tokio::test]
async fn legacy_control_without_builder_keeps_finite_byte_limit_and_unknown_window() {
    let (provider, requests) = ScriptedProvider::new(Vec::new());
    let sink = RecordingSink::default();
    let limits = AgentLimits {
        context_byte_budget: 384,
        ..AgentLimits::default()
    };
    let output = run_agent_with_control(
        &provider,
        &RecordingExecutor::default(),
        request("current prompt ".repeat(45), None),
        Vec::new(),
        AgentRunControl::new(
            limits,
            AnsweringLimits::default(),
            &RecordingApproval::default(),
            &sink,
            CancellationToken::new(),
        ),
    )
    .await
    .unwrap();
    assert!(stopped(&output, BudgetDimension::RequestContextBytes));
    assert!(requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn legacy_sink_entrypoint_keeps_finite_byte_preflight_without_a_window() {
    let (provider, requests) = ScriptedProvider::new(Vec::new());
    let limits = AgentLimits {
        context_byte_budget: 384,
        ..AgentLimits::default()
    };
    let output = run_agent_with_sink(
        &provider,
        &RecordingExecutor::default(),
        request("legacy prompt ".repeat(45), None),
        Vec::new(),
        limits,
        &AllowReadOnlyApproval,
        &NoopEventSink,
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert!(stopped(&output, BudgetDimension::RequestContextBytes));
    assert!(requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn unknown_window_does_not_turn_a_large_output_reserve_into_a_window() {
    let limits = AgentLimits {
        context_byte_budget: 1_024,
        ..AgentLimits::default()
    };
    let (provider, requests) = ScriptedProvider::new(vec![turn("answer", Vec::new())]);
    let output = run(
        &provider,
        &RecordingExecutor::default(),
        request("small request", None),
        Vec::new(),
        control(
            limits,
            &RecordingApproval::default(),
            &RecordingSink::default(),
            CancellationToken::new(),
            preflight(None, u64::MAX),
        ),
    )
    .await
    .unwrap();
    assert_eq!(output.answer, "answer");
    assert!(!stopped(&output, BudgetDimension::RequestContextWindow));
    assert_eq!(requests.lock().unwrap().len(), 1);

    let byte_limits = AgentLimits {
        context_byte_budget: 384,
        ..AgentLimits::default()
    };
    let (provider, requests) = ScriptedProvider::new(Vec::new());
    let refused = run(
        &provider,
        &RecordingExecutor::default(),
        request("oversized prompt ".repeat(45), None),
        Vec::new(),
        control(
            byte_limits,
            &RecordingApproval::default(),
            &RecordingSink::default(),
            CancellationToken::new(),
            preflight(None, u64::MAX),
        ),
    )
    .await
    .unwrap();
    assert!(stopped(&refused, BudgetDimension::RequestContextBytes));
    assert!(!stopped(&refused, BudgetDimension::RequestContextWindow));
    assert!(requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn oversized_nested_schema_refuses_without_dispatch() {
    let (provider, requests) = ScriptedProvider::new(Vec::new());
    let mut tool = definition("query");
    tool.parameters = serde_json::json!({
        "nested": vec!["schema detail".repeat(256); 256]
    });
    let limits = AgentLimits {
        context_byte_budget: 512,
        ..AgentLimits::default()
    };
    let output = run(
        &provider,
        &RecordingExecutor::default(),
        request("short", None),
        vec![tool],
        control(
            limits,
            &RecordingApproval::default(),
            &RecordingSink::default(),
            CancellationToken::new(),
            preflight(None, 32),
        ),
    )
    .await
    .unwrap();
    assert!(stopped(&output, BudgetDimension::RequestContextBytes));
    assert!(requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn accepted_stream_retry_uses_one_request_admission() {
    let failed = vec![
        Ok(ProviderEvent::TextDelta("discarded attempt".into())),
        Err(ProviderError::Request("retry".into())),
    ];
    let (provider, requests) = ScriptedProvider::new(vec![failed, turn("answer", Vec::new())]);
    let sink = RecordingSink::default();
    let output = run(
        &provider,
        &RecordingExecutor::default(),
        request("short", None),
        Vec::new(),
        control(
            AgentLimits::default(),
            &RecordingApproval::default(),
            &sink,
            CancellationToken::new(),
            preflight(None, 32),
        ),
    )
    .await
    .unwrap();
    assert_eq!(output.answer, "answer");
    assert_eq!(requests.lock().unwrap().len(), 2);
    assert_eq!(
        sink.observations
            .lock()
            .unwrap()
            .iter()
            .filter(|observation| { **observation == AnsweringObservation::RequestAdmitted })
            .count(),
        1
    );
}
