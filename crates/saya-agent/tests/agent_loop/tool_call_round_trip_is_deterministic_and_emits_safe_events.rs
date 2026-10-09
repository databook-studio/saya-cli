#[tokio::test]
async fn tool_call_round_trip_is_deterministic_and_emits_safe_events() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let provider = MockProvider {
        responses: Mutex::new(vec![
            ChatResponse::new(ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: "call-1".into(),
                    name: "bounded_sql_query".into(),
                    arguments: serde_json::json!({"sql":"select 1"}),
                }],
                tool_call_id: None,
            }),
            ChatResponse::new(ChatMessage::text("assistant", "There is one result.")),
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
        &AllowReadOnlyApproval,
    )
    .await
    .unwrap();
    assert_eq!(output.answer, "There is one result.");
    assert_eq!(&*calls.lock().unwrap(), &["bounded_sql_query"]);
    assert!(output.used_bounded_sql_query);
    assert_eq!(output.tool_metadata[0].name, "bounded_sql_query");
    assert_eq!(output.tool_metadata[0].status, "completed");
    assert!(output.events.iter().any(|event| matches!(event, saya_agent::AgentEvent::ToolCompleted { summary, .. } if summary.contains("read-only"))));
}
