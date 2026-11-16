/// A turn whose cap lands mid-tool-argument runs no tool at all: the partial
/// call never reaches the executor.
#[tokio::test]
async fn truncated_tool_call_executes_nothing() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = continuation_script(
        &requests,
        false,
        "half an answer",
        vec!["{\"sql\": \"SELECT 1".into()],
    );
    let tool_calls = Arc::new(Mutex::new(Vec::new()));
    let output = run_agent_with_sink(
        &provider,
        &MockTools {
            calls: tool_calls.clone(),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &NoopSink,
        CancellationToken::new(),
    )
    .await
    .expect("the run recovers after the truncated tool call");
    assert_eq!(output.answer, "cooperative answer");
    assert!(
        tool_calls.lock().unwrap().is_empty(),
        "the half-assembled tool call must never execute"
    );
}
