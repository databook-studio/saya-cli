/// One question per turn, and siblings do not ride along: a message pairing a
/// valid ask with other calls ends the turn on the ask; the siblings are
/// answered without executing.
#[tokio::test]
async fn a_valid_ask_refuses_its_sibling_calls_and_ends_the_turn() {
    let provider = ScriptedProvider {
        turns: Mutex::new(vec![ChatMessage {
            role: "assistant".into(),
            content: String::new(),
            tool_calls: vec![
                ToolCall {
                    id: "call-a".into(),
                    name: "schema_discovery".into(),
                    arguments: serde_json::json!({}),
                },
                clarification_call("Which table holds active users?", serde_json::Value::Null),
            ],
            tool_call_id: None,
        }]),
        requests: Mutex::new(0),
    };
    let calls = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::new(Mutex::new(Vec::new()));
    let output = run_agent_with_sink(
        &provider,
        &RecordingTools {
            calls: calls.clone(),
            fail_names: Vec::new(),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &RecordingSink {
            events: captured.clone(),
        },
        CancellationToken::new(),
    )
    .await
    .expect("the turn ends on the clarification");
    assert_eq!(
        &*calls.lock().unwrap(),
        &[REQUEST_CLARIFICATION_TOOL],
        "the sibling must not execute once the turn pauses: {:?}",
        *calls.lock().unwrap()
    );
    assert!(
        output
            .events
            .iter()
            .any(|event| matches!(event, AgentEvent::ClarificationNeeded { .. })),
        "the question is carried: {:?}",
        output.events
    );
}

/// The executor refusing the ask (an executor without the arm) degrades to a
/// tool error the turn continues from — never a silent end, never a fabricated
/// question.
#[tokio::test]
async fn an_executor_refusal_on_the_ask_keeps_the_turn_going() {
    let provider = ScriptedProvider {
        turns: Mutex::new(vec![
            ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![clarification_call(
                    "Which metric should \"active users\" use?",
                    serde_json::Value::Null,
                )],
                tool_call_id: None,
            },
            ChatMessage::text("assistant", "answered without the ask"),
        ]),
        requests: Mutex::new(0),
    };
    let captured = Arc::new(Mutex::new(Vec::new()));
    let output = run_agent_with_sink(
        &provider,
        &RecordingTools {
            calls: Arc::new(Mutex::new(Vec::new())),
            fail_names: vec![REQUEST_CLARIFICATION_TOOL.into()],
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &RecordingSink {
            events: captured.clone(),
        },
        CancellationToken::new(),
    )
    .await
    .expect("an executor refusal degrades to a tool error");
    assert_eq!(output.answer, "answered without the ask");
    assert!(
        !output
            .events
            .iter()
            .any(|event| matches!(event, AgentEvent::ClarificationNeeded { .. })),
        "no question event when the ask did not land: {:?}",
        output.events
    );
}
