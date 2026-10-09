/// When the turn budget is exhausted the run salvages: one final provider
/// call with no tools asks for the best answer from the work already done,
/// and the run returns that answer marked truncated instead of discarding
/// everything with `Limit("turns")`.
#[tokio::test]
async fn salvage_on_turn_budget_returns_best_answer_marked_truncated() {
    let tool_calls = Arc::new(Mutex::new(Vec::new()));
    let provider = MockProvider {
        responses: Mutex::new(vec![
            ChatResponse::new(ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: "c1".into(),
                    name: "schema_discovery".into(),
                    arguments: serde_json::json!({}),
                }],
                tool_call_id: None,
            }),
            ChatResponse::new(ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: "c2".into(),
                    name: "schema_discovery".into(),
                    arguments: serde_json::json!({}),
                }],
                tool_call_id: None,
            }),
            ChatResponse::new(ChatMessage::text(
                "assistant",
                "best effort from gathered results",
            )),
        ]),
    };
    let output = run_agent(
        &provider,
        &MockTools {
            calls: tool_calls.clone(),
        },
        request(),
        definitions(),
        AgentLimits {
            max_turns: Some(2),
            ..AgentLimits::default()
        },
        &AllowReadOnlyApproval,
    )
    .await
    .expect("salvage returns an answer, not an error");
    assert_eq!(output.answer, "best effort from gathered results");
    assert!(output.truncated, "a salvaged run must be marked truncated");
    // The two turns of real work still ran before the budget ran out.
    assert_eq!(
        &*tool_calls.lock().unwrap(),
        &["schema_discovery", "schema_discovery"]
    );
}
