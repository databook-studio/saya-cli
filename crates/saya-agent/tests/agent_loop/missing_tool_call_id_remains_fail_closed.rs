#[tokio::test]
async fn missing_tool_call_id_remains_fail_closed() {
    let provider = MockProvider {
        responses: Mutex::new(vec![ChatResponse::new(ChatMessage {
            role: "assistant".into(),
            content: String::new(),
            tool_calls: vec![ToolCall {
                id: String::new(),
                name: "shell".into(),
                arguments: serde_json::json!({}),
            }],
            tool_call_id: None,
        })]),
    };
    let error = run_agent(
        &provider,
        &MockTools {
            calls: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .unwrap_err();
    // Without a call id there is no way to anchor a well-formed tool result,
    // so the conversation could not continue validly.
    assert!(matches!(error, AgentError::InvalidToolCall));
}
