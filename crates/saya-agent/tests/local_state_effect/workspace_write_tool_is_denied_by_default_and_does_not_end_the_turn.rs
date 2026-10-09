/// A `WriteWorkspace` tool is denied — not executed — when workspace writes
/// are not permitted (the default), surfacing as a `ToolDenied` event whose
/// reason names the workspace gate, and the turn continues to completion.
/// Mirrors the `WriteCandidate` denial test above: the permit gate, not the
/// approval decider, must be the thing that refuses (the tool does not require
/// approval, so the decider is never consulted for it).
#[tokio::test]
async fn workspace_write_tool_is_denied_by_default_and_does_not_end_the_turn() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let provider = OneCallProvider {
        call: ToolCall {
            id: "c1".into(),
            name: "workspace_write".into(),
            arguments: serde_json::json!({}),
        },
        turn: Mutex::new(0),
    };
    let sink = RecordingSink {
        events: events.clone(),
    };
    let token = saya_agent::CancellationToken::new();
    let output = run_agent_with_sink(
        &provider,
        &RecordingExecutor {
            calls: calls.clone(),
        },
        request(),
        vec![workspace_tool()],
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &sink,
        token,
    )
    .await
    .expect("denial is not a turn-ending error");
    assert!(
        calls.lock().unwrap().is_empty(),
        "the tool must not execute when workspace writes are not permitted"
    );
    let denied = events.lock().unwrap().iter().find_map(|event| match event {
        AgentEvent::ToolDenied { name, reason } => Some((name.clone(), reason.clone())),
        _ => None,
    });
    let (name, reason) = denied.expect("a ToolDenied event must be emitted");
    assert_eq!(name, "workspace_write");
    assert!(
        reason.contains("workspace"),
        "the denial must name the workspace gate, not approval, got: {reason}"
    );
    // The turn continued past the denial to a normal completion.
    assert!(
        output
            .events
            .iter()
            .any(|event| matches!(event, AgentEvent::Complete)),
        "the turn must complete, not end on the denial"
    );
    assert_eq!(output.tool_metadata[0].name, "workspace_write");
    assert_eq!(output.tool_metadata[0].status, "denied");
}
