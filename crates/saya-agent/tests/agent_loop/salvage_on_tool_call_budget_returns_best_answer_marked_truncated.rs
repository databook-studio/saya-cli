/// When the tool-call budget would be crossed, the run salvages from the work
/// already done instead of erroring. None of the turn's calls run, and the
/// salvaged answer is returned marked truncated.
#[tokio::test]
async fn salvage_on_tool_call_budget_returns_best_answer_marked_truncated() {
    let tool_calls = Arc::new(Mutex::new(Vec::new()));
    let provider = MockProvider {
        responses: Mutex::new(vec![
            ChatResponse::new(ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![
                    ToolCall {
                        id: "c1".into(),
                        name: "schema_discovery".into(),
                        arguments: serde_json::json!({}),
                    },
                    ToolCall {
                        id: "c2".into(),
                        name: "schema_discovery".into(),
                        arguments: serde_json::json!({}),
                    },
                ],
                tool_call_id: None,
            }),
            ChatResponse::new(ChatMessage::text("assistant", "best effort")),
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
            max_tool_calls: Some(1),
            ..AgentLimits::default()
        },
        &AllowReadOnlyApproval,
    )
    .await
    .expect("salvage returns an answer, not an error");
    assert_eq!(output.answer, "best effort");
    assert!(output.truncated, "a salvaged run must be marked truncated");
    assert!(
        tool_calls.lock().unwrap().is_empty(),
        "no tools run when the turn would cross the budget"
    );
}
