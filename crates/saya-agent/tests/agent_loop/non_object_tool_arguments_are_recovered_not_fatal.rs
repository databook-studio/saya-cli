#[tokio::test]
async fn non_object_tool_arguments_are_recovered_not_fatal() {
    let provider = MockProvider {
        responses: Mutex::new(vec![
            ChatResponse::new(ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: "call-1".into(),
                    name: "schema_discovery".into(),
                    arguments: serde_json::json!("not-an-object"),
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
    assert_eq!(output.answer, "recovered");
    assert!(tools.calls.lock().unwrap().is_empty());
}
