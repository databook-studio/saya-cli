#[tokio::test]
async fn cloud_without_sharing_hides_sql_and_never_sends_rows() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = MockProvider {
        responses: Mutex::new(vec![text_response("schema only")]),
        requests: requests.clone(),
    };
    assert!(!crate::agent::runtime::query_data_allowed(
        saya_config::AiProvider::OpenaiCompatible,
        false
    ));
    let tools = crate::agent::tools::DatabaseTools::definitions(
        crate::agent::runtime::query_data_allowed(saya_config::AiProvider::OpenaiCompatible, false),
        false,
        false,
        false,
        false,
    );
    run_agent(
        &provider,
        &SentinelExecutor,
        request(),
        tools,
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .unwrap();
    let sent = serde_json::to_string(&requests.lock().unwrap()[0]).unwrap();
    assert!(!sent.contains("bounded_sql_query"));
    assert!(!sent.contains("CLOUD_SENTINEL"));
}

#[tokio::test]
async fn cloud_with_sharing_exposes_sql_and_sends_bounded_rows_to_model_only() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = MockProvider {
        responses: Mutex::new(vec![
            ChatResponse::new(ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: "call".into(),
                    name: "bounded_sql_query".into(),
                    arguments: serde_json::json!({"sql":"select 1"}),
                }],
                tool_call_id: None,
            }),
            text_response("done"),
        ]),
        requests: requests.clone(),
    };
    assert!(crate::agent::runtime::query_data_allowed(
        saya_config::AiProvider::OpenaiCompatible,
        true
    ));
    let tools = crate::agent::tools::DatabaseTools::definitions(
        crate::agent::runtime::query_data_allowed(saya_config::AiProvider::OpenaiCompatible, true),
        false,
        false,
        false,
        true,
    );
    run_agent(
        &provider,
        &SentinelExecutor,
        request(),
        tools,
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .unwrap();
    let sent = serde_json::to_string(&*requests.lock().unwrap()).unwrap();
    assert!(sent.contains("bounded_sql_query"));
    assert!(sent.contains("CLOUD_SENTINEL"));
}

#[tokio::test]
async fn cloud_without_sharing_blocks_dispatch_even_for_direct_malicious_call() {
    let tools = crate::agent::tools::DatabaseTools::new(None, 10, false);
    let error = tools
        .execute("bounded_sql_query", serde_json::json!({"sql":"select 1"}))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("data sharing is disabled"));
}
