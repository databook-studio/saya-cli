#[tokio::test]
async fn salvage_refuses_an_invalid_completed_batch_and_preserves_prior_work() {
    let mut invalid = ChatResponse::new(assistant(vec![
        call("duplicate", "schema_discovery", serde_json::json!({})),
        call("duplicate", "schema_discovery", serde_json::json!({})),
    ]));
    invalid.message.content = "INVALID_SALVAGE_ANSWER_SENTINEL".into();
    invalid.usage = Some(TokenUsage::new(7, 2));
    let (provider, requests) = provider(vec![
        ChatResponse::new(assistant(vec![call(
            "completed",
            "schema_discovery",
            serde_json::json!({}),
        )])),
        invalid,
        ChatResponse::new(ChatMessage::text("assistant", "unexpected repair")),
    ]);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let output = run_agent_with_sink(
        &provider,
        &RecordingTools {
            calls: calls.clone(),
        },
        request(),
        definitions(),
        AgentLimits {
            max_turns: Some(1),
            ..AgentLimits::default()
        },
        &AllowReadOnlyApproval,
        &RecordingSink {
            events: events.clone(),
        },
        CancellationToken::new(),
    )
    .await
    .expect("salvage preserves prior work when its completed response is invalid");

    assert!(output.truncated);
    assert!(
        output.answer.is_empty(),
        "invalid salvage content is discarded"
    );
    assert!(
        !output.answer.contains("INVALID_SALVAGE_ANSWER_SENTINEL"),
        "nonempty prose from the rejected salvage response must not escape"
    );
    assert_eq!(&*calls.lock().unwrap(), &["schema_discovery"]);
    assert_eq!(output.usage, TokenUsage::new(7, 2));
    assert!(output.events.iter().any(|event| matches!(
        event,
        AgentEvent::ProviderRecovery {
            phase: ProviderRecoveryPhase::NotRetried,
            reason: ProviderRecoveryReason::ToolCallProtocol,
            ..
        }
    )));
    assert!(output.events.iter().any(|event| matches!(
        event,
        AgentEvent::Usage { usage, .. } if *usage == TokenUsage::new(7, 2)
    )));
    let requests = requests.lock().unwrap();
    assert_eq!(
        requests.len(),
        2,
        "the invalid salvage batch is not retried"
    );
    assert!(requests[1].tools.is_empty(), "salvage does not offer tools");
}

#[tokio::test]
async fn completed_effect_is_not_replayed_when_the_next_batch_has_an_invalid_id() {
    let (provider, requests) = provider(vec![
        ChatResponse::new(assistant(vec![call(
            "completed",
            "schema_discovery",
            serde_json::json!({}),
        )])),
        ChatResponse::new(assistant(vec![call(
            "",
            "schema_discovery",
            serde_json::json!({}),
        )])),
        ChatResponse::new(ChatMessage::text("assistant", "unexpected repair")),
    ]);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let result = run_agent_with_sink(
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
    .await;

    assert!(matches!(result, Err(AgentError::InvalidToolCall)));
    assert_eq!(&*calls.lock().unwrap(), &["schema_discovery"]);
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2, "invalid batch must not retry");
    assert!(
        requests[1]
            .messages
            .iter()
            .any(|message| message.tool_call_id.as_deref() == Some("completed")),
        "the completed result remains in the next request rather than being replayed"
    );
}
