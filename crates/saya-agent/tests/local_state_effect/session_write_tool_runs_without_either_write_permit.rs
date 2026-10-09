/// A session-metadata tool needs neither write permit: `WriteSession` runs
/// under the default limits (both permits off), because no loop gate covers
/// it — the candidate and workspace equality checks name only their own
/// variants, and the approval predicate admits it.
#[tokio::test]
async fn session_write_tool_runs_without_either_write_permit() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let provider = OneCallProvider {
        call: ToolCall {
            id: "c1".into(),
            name: "session_task_list".into(),
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
        vec![session_tool()],
        AgentLimits::default(),
        &AllowApproval,
        &sink,
        token,
    )
    .await
    .expect("run completes");
    assert_eq!(&*calls.lock().unwrap(), &["session_task_list"]);
    assert!(
        !output
            .events
            .iter()
            .any(|event| matches!(event, AgentEvent::ToolDenied { .. })),
        "a session-metadata tool must not be denied by either write gate"
    );
    assert_eq!(output.tool_metadata[0].status, "completed");
}
