use super::*;

#[tokio::test]
async fn failed_candidate_receipt_survives_later_sqlite_answer_once() {
    let root = temp_root();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let (base_url, source) = candidate_source(Arc::clone(&requests));
    let runtime = runtime_with_sqlite(&root, base_url).await;
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink(Arc::clone(&events));
    let cancellation = CancellationToken::new();
    let scope = QuestionScope::new(&runtime, cancellation.clone());

    let output = tokio::time::timeout(
        Duration::from_secs(5),
        run_with_candidates(
            &runtime,
            "Count the orders.",
            ApprovalPolicy::ReadOnly,
            false,
            false,
            PromptOverrides::default(),
            Vec::new(),
            &sink,
            scope,
            None,
            None,
            None,
            None,
            AgentMode::Build,
            2,
        ),
    )
    .await
    .expect("two bounded candidate attempts settle")
    .expect("the SQLite-backed candidate answers");

    source
        .join()
        .expect("candidate source thread joins")
        .expect("candidate source serves the expected requests");
    let seen = requests.lock().expect("request log lock");
    assert_eq!(seen.len(), 3, "one rejected attempt, then tool and answer");
    assert!(seen[0].contains("bounded_sql_query"));
    assert!(seen[1].contains("bounded_sql_query"));
    let third_request: serde_json::Value =
        serde_json::from_str(&seen[2]).expect("third provider request is JSON");
    let tool_messages = third_request["messages"]
        .as_array()
        .expect("third provider request carries messages")
        .iter()
        .filter(|message| message["role"] == "tool" && message["tool_call_id"] == "call-sql")
        .collect::<Vec<_>>();
    assert_eq!(tool_messages.len(), 1, "one paired SQLite tool message");
    let tool_result: serde_json::Value = serde_json::from_str(
        tool_messages[0]["content"]
            .as_str()
            .expect("paired tool message carries JSON content"),
    )
    .expect("paired tool result is JSON");
    assert!(tool_result.get("error").is_none(), "SQLite tool succeeded");
    assert_eq!(tool_result["columns"], serde_json::json!(["id"]));
    assert_eq!(tool_result["row_count"], 1);
    assert_eq!(tool_result["truncated"], false);
    assert_eq!(tool_result["rows"], serde_json::json!([[1]]));
    assert_eq!(output.answer, "The query returned one.");
    assert_eq!(
        output.usage.input_tokens, 51,
        "11 + 17 + 23 paid input tokens"
    );
    assert_eq!(
        output.usage.output_tokens, 61,
        "13 + 19 + 29 paid output tokens"
    );
    let events = events.lock().expect("event log lock");
    assert!(
        events
            .iter()
            .any(|event| matches!(event, AgentEvent::ProviderRecovery { .. }))
    );
    assert!(events.iter().any(|event| matches!(
        event,
        AgentEvent::ToolCompleted { name, .. } if name == "bounded_sql_query"
    )));
    let _ = std::fs::remove_dir_all(root);
}
