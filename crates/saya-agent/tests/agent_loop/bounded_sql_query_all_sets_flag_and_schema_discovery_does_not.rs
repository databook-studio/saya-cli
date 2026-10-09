#[tokio::test]
async fn bounded_sql_query_all_sets_flag_and_schema_discovery_does_not() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let provider = MockProvider {
        responses: Mutex::new(vec![
            ChatResponse::new(ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: "call-1".into(),
                    name: "bounded_sql_query_all".into(),
                    arguments: serde_json::json!({"sql":"select 1"}),
                }],
                tool_call_id: None,
            }),
            ChatResponse::new(ChatMessage::text("assistant", "Results across databases.")),
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
    assert_eq!(output.answer, "Results across databases.");
    assert_eq!(&*calls.lock().unwrap(), &["bounded_sql_query_all"]);
    assert!(output.used_bounded_sql_query);

    let calls_schema = Arc::new(Mutex::new(Vec::new()));
    let provider_schema = MockProvider {
        responses: Mutex::new(vec![
            ChatResponse::new(ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: "call-2".into(),
                    name: "schema_discovery".into(),
                    arguments: serde_json::json!({}),
                }],
                tool_call_id: None,
            }),
            ChatResponse::new(ChatMessage::text("assistant", "Discovered schema.")),
        ]),
    };
    let output_schema = run_agent(
        &provider_schema,
        &MockTools {
            calls: calls_schema.clone(),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .unwrap();
    assert_eq!(output_schema.answer, "Discovered schema.");
    assert_eq!(&*calls_schema.lock().unwrap(), &["schema_discovery"]);
    assert!(!output_schema.used_bounded_sql_query);
}
