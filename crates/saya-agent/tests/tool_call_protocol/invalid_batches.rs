#[tokio::test]
async fn blank_or_duplicate_ids_refuse_the_whole_mixed_batch_before_any_effect() {
    for (label, calls) in [
        (
            "blank",
            vec![
                call("valid", "schema_discovery", serde_json::json!({})),
                call("", "schema_discovery", serde_json::json!({})),
            ],
        ),
        (
            "duplicate",
            vec![
                call("same", "schema_discovery", serde_json::json!({})),
                call("same", "schema_discovery", serde_json::json!({})),
            ],
        ),
    ] {
        let mut response = ChatResponse::new(assistant(calls));
        response.usage = Some(TokenUsage::new(11, 2));
        let (provider, requests) = provider(vec![
            response,
            ChatResponse::new(ChatMessage::text("assistant", "unexpected repair")),
        ]);
        let calls = Arc::new(Mutex::new(Vec::new()));
        let events = Arc::new(Mutex::new(Vec::new()));
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
                events: events.clone(),
            },
            CancellationToken::new(),
        )
        .await;

        assert!(
            matches!(result, Err(AgentError::InvalidToolCall)),
            "{label} IDs must reject the completed batch: {result:?}"
        );
        assert!(
            calls.lock().unwrap().is_empty(),
            "{label}: no call may execute"
        );
        assert_eq!(
            requests.lock().unwrap().len(),
            1,
            "{label}: no repair retry"
        );
        let events = events.lock().unwrap();
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, AgentEvent::ToolRequested { .. })),
            "{label}: no prefix tool request may escape: {events:?}"
        );
        assert!(
            events.iter().any(|event| matches!(
                event,
                AgentEvent::Usage { usage, .. }
                    if *usage == TokenUsage::new(11, 2)
            )),
            "{label}: reported provider usage remains known: {events:?}"
        );
        assert!(
            events.iter().any(|event| matches!(
                event,
                AgentEvent::ProviderRecovery {
                    phase: ProviderRecoveryPhase::NotRetried,
                    reason: ProviderRecoveryReason::ToolCallProtocol,
                    attempt: 1,
                    limit: 4,
                }
            )),
            "{label}: the refusal needs one bounded typed reason: {events:?}"
        );
    }
}

#[tokio::test]
async fn invalid_ids_preempt_each_shortcut_before_its_side_actions() {
    for (label, shortcut, expected) in [
        (
            "clarification",
            call(
                "same",
                REQUEST_CLARIFICATION_TOOL,
                serde_json::json!({"question": "Which metric?"}),
            ),
            "clarification",
        ),
        (
            "designation",
            call(
                "same",
                DESIGNATE_ANSWER_TOOL,
                serde_json::json!({"sql": "SELECT 1"}),
            ),
            "designation",
        ),
    ] {
        let (provider, _) = provider(vec![
            ChatResponse::new(assistant(vec![
                shortcut,
                call("same", "schema_discovery", serde_json::json!({})),
            ])),
            ChatResponse::new(ChatMessage::text("assistant", "unexpected repair")),
        ]);
        let calls = Arc::new(Mutex::new(Vec::new()));
        let events = Arc::new(Mutex::new(Vec::new()));
        let result = run_agent_with_sink(
            &provider,
            &RecordingTools {
                calls: calls.clone(),
            },
            request(),
            definitions_with_shortcuts(),
            AgentLimits::default(),
            &AllowReadOnlyApproval,
            &RecordingSink {
                events: events.clone(),
            },
            CancellationToken::new(),
        )
        .await;

        assert!(
            matches!(result, Err(AgentError::InvalidToolCall)),
            "{label}: duplicate IDs must reject the batch"
        );
        assert!(
            calls.lock().unwrap().is_empty(),
            "{label}: no tool executes"
        );
        let events = events.lock().unwrap();
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, AgentEvent::ToolRequested { .. })),
            "{label}: no tool request may escape: {events:?}"
        );
        match expected {
            "clarification" => assert!(
                !events
                    .iter()
                    .any(|event| matches!(event, AgentEvent::ClarificationNeeded { .. })),
                "{label}: the clarification must not land: {events:?}"
            ),
            "designation" => assert!(
                !events
                    .iter()
                    .any(|event| matches!(event, AgentEvent::AnswerDesignated { .. })),
                "{label}: the designation must not land: {events:?}"
            ),
            _ => unreachable!("the table fixes the only shortcut cases"),
        }
    }
}
