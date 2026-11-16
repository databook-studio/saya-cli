#[tokio::test]
async fn cancellation_blocks_tool_execution_and_terminal_events() {
    let token = CancellationToken::new();
    let events = Arc::new(Mutex::new(Vec::new()));
    let calls = Arc::new(Mutex::new(Vec::new()));
    let error = run_agent_with_sink(
        &StreamingToolProvider,
        &MockTools {
            calls: calls.clone(),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &CancellingSink {
            token: token.clone(),
            events: events.clone(),
        },
        token,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, AgentError::Cancelled));
    assert!(calls.lock().unwrap().is_empty());
    assert!(!events.lock().unwrap().iter().any(|event| matches!(
        event,
        AgentEvent::ToolCompleted { .. } | AgentEvent::Complete
    )));
}
