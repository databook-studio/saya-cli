#[tokio::test]
async fn tool_call_limit_salvages_before_any_unbounded_execution() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let provider = MockProvider {
        responses: Mutex::new(vec![
            ChatResponse::new(ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: "call-1".into(),
                    name: "bounded_sql_query".into(),
                    arguments: serde_json::json!({}),
                }],
                tool_call_id: None,
            }),
            ChatResponse::new(ChatMessage::text("assistant", "salvaged answer")),
        ]),
    };
    let output = run_agent(
        &provider,
        &MockTools {
            calls: calls.clone(),
        },
        request(),
        definitions(),
        AgentLimits {
            max_turns: Some(1),
            max_tool_calls: Some(0),
            permit_candidate_writes: false,
            ..AgentLimits::default()
        },
        &AllowReadOnlyApproval,
    )
    .await
    .expect("a crossed budget salvages rather than erroring");
    assert_eq!(output.answer, "salvaged answer");
    assert!(output.truncated, "a salvaged run is marked truncated");
    assert!(
        calls.lock().unwrap().is_empty(),
        "the budget-crossing tool must not execute"
    );
}
