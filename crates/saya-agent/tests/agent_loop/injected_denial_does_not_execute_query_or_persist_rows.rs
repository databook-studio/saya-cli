#[tokio::test]
async fn injected_denial_does_not_execute_query_or_persist_rows() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let provider = MockProvider {
        responses: Mutex::new(vec![
            ChatResponse::new(ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: "call-1".into(),
                    name: "bounded_sql_query".into(),
                    arguments: serde_json::json!({"sql":"select secret"}),
                }],
                tool_call_id: None,
            }),
            ChatResponse::new(ChatMessage::text("assistant", "The query was denied.")),
        ]),
    };
    let output = run_agent(
        &provider,
        &MockTools {
            calls: calls.clone(),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &DenyApproval,
    )
    .await
    .unwrap();
    assert!(calls.lock().unwrap().is_empty());
    assert!(!output.used_bounded_sql_query);
    assert!(
        output
            .events
            .iter()
            .any(|event| matches!(event, saya_agent::AgentEvent::ToolDenied { .. }))
    );
    assert!(!output.answer.contains("secret"));
}
