#[tokio::test]
async fn valid_ids_allow_registered_clarification_and_designation_shortcuts() {
    let clarification_events = Arc::new(Mutex::new(Vec::new()));
    let clarification_calls = Arc::new(Mutex::new(Vec::new()));
    let (clarification_provider, clarification_requests) = provider(vec![
        ChatResponse::new(assistant(vec![call(
            "ask",
            REQUEST_CLARIFICATION_TOOL,
            serde_json::json!({"question": "Which metric?"}),
        )])),
        ChatResponse::new(ChatMessage::text("assistant", "unexpected repair")),
    ]);
    let clarification = run_agent_with_sink(
        &clarification_provider,
        &RecordingTools {
            calls: clarification_calls.clone(),
        },
        request(),
        definitions_with_shortcuts(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &RecordingSink {
            events: clarification_events.clone(),
        },
        CancellationToken::new(),
    )
    .await
    .expect("a valid registered clarification lands");
    assert_eq!(
        &*clarification_calls.lock().unwrap(),
        &[REQUEST_CLARIFICATION_TOOL]
    );
    assert!(clarification.events.iter().any(
        |event| matches!(event, AgentEvent::ClarificationNeeded { question, .. } if question == "Which metric?")
    ));
    assert_eq!(clarification_requests.lock().unwrap().len(), 1);

    let designation_events = Arc::new(Mutex::new(Vec::new()));
    let designation_calls = Arc::new(Mutex::new(Vec::new()));
    let (designation_provider, _) = provider(vec![
        ChatResponse::new(ChatMessage {
            role: "assistant".into(),
            content: "The answer is one.".into(),
            tool_calls: vec![call(
                "designate",
                DESIGNATE_ANSWER_TOOL,
                serde_json::json!({"sql": "SELECT 1"}),
            )],
            tool_call_id: None,
        }),
        ChatResponse::new(ChatMessage::text("assistant", "unexpected repair")),
    ]);
    let designation = run_agent_with_sink(
        &designation_provider,
        &RecordingTools {
            calls: designation_calls.clone(),
        },
        request(),
        definitions_with_shortcuts(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &RecordingSink {
            events: designation_events,
        },
        CancellationToken::new(),
    )
    .await
    .expect("a valid registered designation lands");
    assert_eq!(designation.answer_sql.as_deref(), Some("SELECT 1"));
    assert!(designation_calls.lock().unwrap().is_empty());
    assert!(
        designation.events.iter().any(
            |event| matches!(event, AgentEvent::AnswerDesignated { sql } if sql == "SELECT 1")
        )
    );
}
