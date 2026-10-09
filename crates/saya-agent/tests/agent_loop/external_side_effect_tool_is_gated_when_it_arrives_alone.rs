/// The gated tool arriving alone takes the sequential path: it must be
/// denied, not executed. Against the code before parallel tool calls the sequential path ignored
/// `external_side_effect`, so this assertion fails there (the tool ran).
#[tokio::test]
async fn external_side_effect_tool_is_gated_when_it_arrives_alone() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let provider = MockProvider {
        responses: Mutex::new(vec![
            ChatResponse::new(ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: "call-1".into(),
                    name: "open_browser".into(),
                    arguments: serde_json::json!({}),
                }],
                tool_call_id: None,
            }),
            ChatResponse::new(ChatMessage::text("assistant", "done")),
        ]),
    };
    let sink = RecordingSink {
        events: events.clone(),
    };
    let output = run_agent_with_sink(
        &provider,
        &MockTools {
            calls: calls.clone(),
        },
        request(),
        vec![external_side_effect_without_approval_tool()],
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &sink,
        CancellationToken::new(),
    )
    .await
    .expect("a denial is not a turn-ending error");
    assert_open_browser_gated(&calls.lock().unwrap().clone(), &events.lock().unwrap());
    assert_eq!(output.tool_metadata[0].name, "open_browser");
    assert_eq!(output.tool_metadata[0].status, "denied");
}
