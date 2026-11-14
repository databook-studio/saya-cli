/// With reasoning absent, output is byte-identical to before reasoning existed.
/// A provider that streams an answer and no `ReasoningDelta` produces no
/// `ReasoningText` event — the new variant is silent when there is nothing to
/// carry, so a non-reasoning provider's event stream is unchanged.
#[tokio::test]
async fn a_stream_without_reasoning_emits_no_reasoning_event() {
    let events: Arc<Mutex<Vec<AgentEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let provider = ReasoningProvider {
        events: vec![
            ProviderEvent::TextDelta("the answer".into()),
            ProviderEvent::Done,
        ],
    };
    let sink = RecordingSink {
        events: events.clone(),
    };
    let output = run_agent_with_sink(
        &provider,
        &MockTools {
            calls: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &sink,
        CancellationToken::new(),
    )
    .await
    .expect("run completes");
    assert_eq!(output.answer, "the answer");
    let events = events.lock().unwrap().clone();
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, AgentEvent::ReasoningText { .. })),
        "a reasoning-free stream must emit no ReasoningText: {events:?}"
    );
    // The answer and the terminator still arrive — the event shape a
    // non-reasoning provider has always produced.
    assert!(
        events.iter().any(
            |event| matches!(event, AgentEvent::AssistantText { text } if text == "the answer")
        ),
        "the answer must still stream: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, AgentEvent::Complete)),
        "the terminator must still arrive: {events:?}"
    );
}
