/// A provider that reports no usage emits no usage event: absence on the
/// stream means "unknown", and a fabricated `0/0` line would read as a free
/// call the provider never accounted.
#[tokio::test]
async fn a_silent_provider_emits_no_usage_event() {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let output = run_agent_with_sink(
        &MockProvider {
            responses: Mutex::new(vec![ChatResponse::new(ChatMessage::text(
                "assistant",
                "the answer",
            ))]),
        },
        &MockTools {
            calls: Arc::new(Mutex::new(Vec::new())),
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
    .unwrap();
    assert_eq!(output.answer, "the answer");
    let events = captured.lock().unwrap();
    assert!(
        events
            .iter()
            .all(|event| !matches!(event, AgentEvent::Usage { .. })),
        "a silent provider must not produce a usage event: {events:?}"
    );
}
