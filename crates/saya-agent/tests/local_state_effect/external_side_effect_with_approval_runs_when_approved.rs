/// The external-side-effect gate must NOT double-deny a tool
/// that also requires approval and was approved — a probe with the shape
/// `external_side_effect: true, requires_approval: true`. Approval is
/// the real gate there; when it is granted, the tool runs. Read-only
/// approval's denial of such tools is pinned in `approval_matrix.rs`, so this
/// uses a decider that grants.
#[tokio::test]
async fn external_side_effect_with_approval_runs_when_approved() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let provider = OneCallProvider {
        call: ToolCall {
            id: "c1".into(),
            name: "external_effect_probe".into(),
            arguments: serde_json::json!({}),
        },
        turn: Mutex::new(0),
    };
    let sink = RecordingSink {
        events: events.clone(),
    };
    let external_effect_probe = ToolDefinition {
        name: "external_effect_probe".into(),
        description: "visualise a query".into(),
        read_only: true,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: true,
            requires_approval: true,
            local_state: LocalStateEffect::None,
        },
        concurrency: saya_agent::ToolConcurrency::Serial,
        completion: None,
    };
    let _ = run_agent_with_sink(
        &provider,
        &RecordingExecutor {
            calls: calls.clone(),
        },
        request(),
        vec![external_effect_probe],
        AgentLimits::default(),
        &AllowApproval,
        &sink,
        saya_agent::CancellationToken::new(),
    )
    .await
    .expect("run completes");
    assert_eq!(&*calls.lock().unwrap(), &["external_effect_probe"]);
    assert!(
        !events
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, AgentEvent::ToolDenied { .. })),
        "an approved external-side-effect tool must run, not be denied"
    );
}
