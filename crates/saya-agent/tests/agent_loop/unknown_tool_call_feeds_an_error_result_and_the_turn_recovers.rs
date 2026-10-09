#[tokio::test]
async fn unknown_tool_call_feeds_an_error_result_and_the_turn_recovers() {
    let provider = MockProvider {
        responses: Mutex::new(vec![
            ChatResponse::new(ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: "call-1".into(),
                    name: "shell".into(),
                    arguments: serde_json::json!({}),
                }],
                tool_call_id: None,
            }),
            ChatResponse::new(ChatMessage::text("assistant", "recovered")),
        ]),
    };
    let tools = MockTools {
        calls: Arc::new(Mutex::new(Vec::new())),
    };
    let output = run_agent(
        &provider,
        &tools,
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .unwrap();
    // The hallucinated tool never executed, the model saw an error result,
    // and it corrected itself instead of the run aborting.
    assert_eq!(output.answer, "recovered");
    assert!(tools.calls.lock().unwrap().is_empty());
    assert!(
        output
            .tool_metadata
            .iter()
            .any(|item| item.name == "shell" && item.status == "failed")
    );
}
