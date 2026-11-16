/// The same `WriteCandidate` tool executes normally when the permission is on.
#[tokio::test]
async fn write_candidate_tool_runs_when_candidate_writes_are_permitted() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let provider = OneCallProvider {
        call: ToolCall {
            id: "c1".into(),
            name: "remember_candidate".into(),
            arguments: serde_json::json!({}),
        },
        turn: Mutex::new(0),
    };
    let sink = RecordingSink {
        events: events.clone(),
    };
    let token = saya_agent::CancellationToken::new();
    let limits = AgentLimits {
        permit_candidate_writes: true,
        ..AgentLimits::default()
    };
    let output = run_agent_with_sink(
        &provider,
        &RecordingExecutor {
            calls: calls.clone(),
        },
        request(),
        vec![candidate_tool()],
        limits,
        &AllowReadOnlyApproval,
        &sink,
        token,
    )
    .await
    .expect("run completes");
    assert_eq!(&*calls.lock().unwrap(), &["remember_candidate"]);
    assert!(
        !output
            .events
            .iter()
            .any(|event| matches!(event, AgentEvent::ToolDenied { .. })),
        "no denial when candidate writes are permitted"
    );
    assert_eq!(output.tool_metadata[0].status, "completed");
}
