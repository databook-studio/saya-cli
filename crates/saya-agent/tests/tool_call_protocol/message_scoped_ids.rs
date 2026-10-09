#[tokio::test]
async fn the_same_id_in_separate_assistant_messages_executes_once_per_message() {
    let (provider, requests) = provider(vec![
        ChatResponse::new(assistant(vec![call(
            "reused",
            "schema_discovery",
            serde_json::json!({}),
        )])),
        ChatResponse::new(assistant(vec![call(
            "reused",
            "schema_discovery",
            serde_json::json!({}),
        )])),
        ChatResponse::new(ChatMessage::text("assistant", "done")),
    ]);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let output = run_agent_with_sink(
        &provider,
        &RecordingTools {
            calls: calls.clone(),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &RecordingSink {
            events: Arc::new(Mutex::new(Vec::new())),
        },
        CancellationToken::new(),
    )
    .await
    .expect("IDs are unique within each message, not across turns");

    assert_eq!(output.answer, "done");
    assert_eq!(
        &*calls.lock().unwrap(),
        &["schema_discovery", "schema_discovery"]
    );
    assert_eq!(requests.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn valid_ids_with_invalid_calls_each_reply_once_without_execution() {
    let (provider, requests) = provider(vec![
        ChatResponse::new(assistant(vec![
            call("unknown", "not_registered", serde_json::json!({})),
            call(
                "malformed",
                "schema_discovery",
                serde_json::json!("not an object"),
            ),
        ])),
        ChatResponse::new(ChatMessage::text("assistant", "corrected")),
    ]);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let output = run_agent_with_sink(
        &provider,
        &RecordingTools {
            calls: calls.clone(),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &RecordingSink {
            events: Arc::new(Mutex::new(Vec::new())),
        },
        CancellationToken::new(),
    )
    .await
    .expect("valid IDs keep invalid calls recoverable");

    assert_eq!(output.answer, "corrected");
    assert!(
        calls.lock().unwrap().is_empty(),
        "invalid calls never execute"
    );
    let requests = requests.lock().unwrap();
    let replies = requests[1]
        .messages
        .iter()
        .filter(|message| message.role == "tool")
        .filter_map(|message| {
            message
                .tool_call_id
                .as_deref()
                .map(|id| (id, &message.content))
        })
        .collect::<Vec<_>>();
    assert_eq!(replies.len(), 2);
    assert!(
        replies
            .iter()
            .any(|(id, content)| *id == "unknown" && content.contains("unknown tool")),
        "the unknown call has one non-executed error reply: {replies:?}"
    );
    assert!(
        replies.iter().any(|(id, content)| {
            *id == "malformed" && content.contains("arguments must be a JSON object")
        }),
        "the malformed call has one non-executed error reply: {replies:?}"
    );
}
