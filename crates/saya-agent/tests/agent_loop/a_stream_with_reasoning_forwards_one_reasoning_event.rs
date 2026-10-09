/// the turn's captured chain-of-thought is
/// forwarded onto the event stream as `ReasoningText`. A provider that streams
/// reasoning deltas produces exactly one `ReasoningText` carrying the
/// concatenated text — proving the `_reasoning` value once dropped is now a
/// forward, and that the headless renderer's silence is a display
/// decision, not a capture gap. The reasoning event reaches the sink; what the
/// sink's renderer does with it is the renderer's call.
#[tokio::test]
async fn a_stream_with_reasoning_forwards_one_reasoning_event() {
    let events: Arc<Mutex<Vec<AgentEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let provider = ReasoningProvider {
        events: vec![
            ProviderEvent::ReasoningDelta("first I considered ".into()),
            ProviderEvent::ReasoningDelta("the time column".into()),
            ProviderEvent::TextDelta("ok".into()),
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
    assert_eq!(output.answer, "ok");
    let events = events.lock().unwrap().clone();
    let reasoning: Vec<&AgentEvent> = events
        .iter()
        .filter(|event| matches!(event, AgentEvent::ReasoningText { .. }))
        .collect();
    assert_eq!(
        reasoning.len(),
        1,
        "exactly one ReasoningText event (the accumulated turn): {events:?}"
    );
    match reasoning[0] {
        AgentEvent::ReasoningText { text } => assert_eq!(
            text, "first I considered the time column",
            "deltas concatenate into one turn string: {events:?}"
        ),
        _ => unreachable!(),
    }
    // The answer still arrives alongside it — reasoning does not displace the
    // answer on the event stream.
    assert!(
        events
            .iter()
            .any(|event| matches!(event, AgentEvent::AssistantText { text } if text == "ok")),
        "the answer must still stream: {events:?}"
    );
}
