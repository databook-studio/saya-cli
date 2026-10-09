/// The core invariant: a valid `request_clarification` call emits
/// `ClarificationNeeded`, feeds a short tool result, and ENDS the turn — the
/// scripted provider is never called a second time, and the run returns
/// `Ok` rather than an error.
#[tokio::test]
async fn a_clarification_call_ends_the_turn_and_emits_the_event() {
    let provider = ScriptedProvider {
        turns: Mutex::new(vec![ChatMessage {
            role: "assistant".into(),
            content: String::new(),
            tool_calls: vec![clarification_call(
                "Which metric should \"active users\" use?",
                serde_json::json!([
                    "sessions in the last 30 days",
                    "purchases in the last 90 days"
                ]),
            )],
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
    .expect("the turn ends on the clarification, never in an error");
    assert_eq!(
        *provider.requests.lock().unwrap(),
        1,
        "no further provider call after the clarification"
    );
    let events = captured.lock().unwrap();
    assert!(
        events.iter().any(|event| matches!(
            event,
            AgentEvent::ClarificationNeeded { question, options }
                if question == "Which metric should \"active users\" use?"
                    && *options == vec![
                        "sessions in the last 30 days".to_string(),
                        "purchases in the last 90 days".to_string(),
                    ]
        )),
        "the question and its options must be carried on the event: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, AgentEvent::ToolRequested { name, .. } if name == REQUEST_CLARIFICATION_TOOL)),
        "the ask is surfaced like any tool call: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, AgentEvent::Complete)),
        "the stream terminates: {events:?}"
    );
    assert_eq!(
        &*calls.lock().unwrap(),
        &[REQUEST_CLARIFICATION_TOOL],
        "the ask is executed through the executor (the short tool result), nothing else"
    );
    assert!(
        output
            .tool_metadata
            .iter()
            .any(|item| item.name == REQUEST_CLARIFICATION_TOOL && item.status == "completed"),
        "the completed ask is recorded: {:?}",
        output.tool_metadata
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, AgentEvent::ClarificationNeeded { .. }))
            .count(),
        1,
        "exactly one clarification event: {events:?}"
    );
}

/// A malformed call — a question over the character bound — is a tool error:
/// the model sees the reason, the executor never runs, and the turn
/// continues to a normal answer.
#[tokio::test]
async fn an_invalid_clarification_call_feeds_an_error_and_the_turn_continues() {
    let provider = ScriptedProvider {
        turns: Mutex::new(vec![
            ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![clarification_call("x".repeat(301), serde_json::Value::Null)],
                tool_call_id: None,
            },
            ChatMessage::text("assistant", "assumed nothing, answered"),
        ]),
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
    .expect("a malformed call is recoverable, never fatal");
    assert_eq!(output.answer, "assumed nothing, answered");
    assert_eq!(
        *provider.requests.lock().unwrap(),
        2,
        "the turn continued to a second provider call"
    );
    assert!(
        calls.lock().unwrap().is_empty(),
        "a malformed ask must not execute: {:?}",
        *calls.lock().unwrap()
    );
    assert!(
        output
            .tool_metadata
            .iter()
            .any(|item| item.name == REQUEST_CLARIFICATION_TOOL && item.status == "failed"),
        "the failed call is recorded: {:?}",
        output.tool_metadata
    );
    let events = captured.lock().unwrap();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, AgentEvent::ToolCompleted { summary, .. } if summary.contains("failed validation"))),
        "the tool error is surfaced: {events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, AgentEvent::ClarificationNeeded { .. })),
        "no clarification event for a malformed ask: {events:?}"
    );
}

/// A refused ask does not land: it keeps the decider's custom reason, never
/// executes, and lets the provider answer normally on the following turn.
#[tokio::test]
async fn a_denied_clarification_keeps_the_custom_refusal_and_continues() {
    let provider = ScriptedProvider {
        turns: Mutex::new(vec![
            ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![clarification_call(
                    "Which metric should active users use?",
                    serde_json::Value::Null,
                )],
                tool_call_id: None,
            },
            ChatMessage::text("assistant", "answered without asking"),
        ]),
        requests: Mutex::new(0),
    };
    let calls = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut definition = clarification_definition();
    definition.effect.requires_approval = true;
    let output = run_agent_with_sink(
        &provider,
        &RecordingTools {
            calls: calls.clone(),
            fail_names: Vec::new(),
        },
        request(),
        vec![definition],
        AgentLimits::default(),
        &CustomDenyApproval,
        &RecordingSink {
            events: events.clone(),
        },
        CancellationToken::new(),
    )
    .await
    .expect("a denied ask feeds a result and the turn continues");

    assert_eq!(output.answer, "answered without asking");
    assert_eq!(output.tool_metadata[0].status, "denied");
    assert!(
        calls.lock().unwrap().is_empty(),
        "a denied ask must not execute"
    );
    let events = events.lock().unwrap();
    assert!(events.iter().any(|event| matches!(
        event,
        AgentEvent::ToolDenied { reason, .. }
            if reason == "questions require an approved investigation plan"
    )));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, AgentEvent::ClarificationNeeded { .. })),
        "a refused ask must not ask the user: {events:?}"
    );
}
