/// A read-only local-state tool is unaffected by the permission either way.
#[tokio::test]
async fn read_local_state_tool_is_unaffected_by_the_candidate_permission() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let provider = OneCallProvider {
        call: ToolCall {
            id: "c1".into(),
            name: "contract_search".into(),
            arguments: serde_json::json!({}),
        },
        turn: Mutex::new(0),
    };
    let sink = RecordingSink {
        events: events.clone(),
    };
    let token = saya_agent::CancellationToken::new();
    let read_tool = ToolDefinition {
        name: "contract_search".into(),
        description: "reads local contracts".into(),
        read_only: true,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::Read,
        },
        concurrency: saya_agent::ToolConcurrency::Serial,
        completion: None,
    };
    // Default (not permitted) — a Read tool must still run.
    let _ = run_agent_with_sink(
        &provider,
        &RecordingExecutor {
            calls: calls.clone(),
        },
        request(),
        vec![read_tool.clone()],
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &sink,
        token,
    )
    .await
    .expect("run completes");
    assert_eq!(&*calls.lock().unwrap(), &["contract_search"]);
    assert!(
        !events
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, AgentEvent::ToolDenied { .. })),
        "a Read tool must not be denied by the candidate-write guard"
    );
}
