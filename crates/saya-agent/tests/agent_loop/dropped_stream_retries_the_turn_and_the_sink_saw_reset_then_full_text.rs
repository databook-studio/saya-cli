#[tokio::test]
async fn dropped_stream_retries_the_turn_and_the_sink_saw_reset_then_full_text() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = DropThenAnswerProvider {
        requests: requests.clone(),
    };
    let events = Arc::new(Mutex::new(Vec::new()));
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
    .expect("the run completes after the retried turn");
    assert_eq!(
        output.answer, "The answer is 42.",
        "the retried answer is the run's answer"
    );

    // The sink saw reset-then-full text: the partial attempt's deltas came
    // before the reset, the full retry after it — never concatenated.
    let seen = events.lock().unwrap().clone();
    // Every attempt announces itself, the first and the retried one alike.
    // A sink rolls back to this boundary, so a missing one would leave the
    // TUI reaching for the request chapter instead and erasing an earlier
    // step's delivered evidence (re-audit R02).
    assert_eq!(
        seen.iter()
            .filter(|event| matches!(event, AgentEvent::TurnStarted))
            .count(),
        2,
        "one TurnStarted before the first attempt and one before the retry:\n{seen:#?}"
    );
    assert!(
        matches!(seen.first(), Some(AgentEvent::TurnStarted)),
        "the attempt boundary precedes anything the attempt can produce"
    );
    let split = seen
        .iter()
        .position(|event| matches!(event, AgentEvent::TurnReset))
        .expect("the sink saw a TurnReset before the retry");
    assert!(
        matches!(
            seen.get(split + 1),
            Some(AgentEvent::ProviderRecovery { .. })
        ),
        "the reset clears discarded text before typed retry progress"
    );
    assert!(
        matches!(seen.get(split + 2), Some(AgentEvent::TurnStarted)),
        "the retried attempt announces its own boundary after recovery progress"
    );
    let partial: String = seen[..split]
        .iter()
        .filter_map(|event| match event {
            AgentEvent::AssistantText { text } => Some(text.clone()),
            _ => None,
        })
        .collect();
    let full: String = seen[split + 1..]
        .iter()
        .filter_map(|event| match event {
            AgentEvent::AssistantText { text } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        partial, "The answ",
        "the partial attempt's deltas came before the reset: {seen:?}"
    );
    assert_eq!(
        full, "The answer is 42.",
        "the retry re-emitted the full answer after the reset: {seen:?}"
    );
    assert_eq!(
        seen.iter()
            .filter(|event| matches!(event, AgentEvent::TurnReset))
            .count(),
        1,
        "exactly one reset for one retry: {seen:?}"
    );
    // Folding the stream — append deltas, clear at resets — reproduces the
    // answer exactly, so no text was ever duplicated.
    let mut folded = String::new();
    for event in &seen {
        match event {
            AgentEvent::AssistantText { text } => folded.push_str(text),
            AgentEvent::TurnReset => folded.clear(),
            _ => {}
        }
    }
    assert_eq!(folded, "The answer is 42.", "no duplicated text: {seen:?}");

    // The retry re-sent the conversation exactly as it stood at turn start —
    // no partial assistant message entered it.
    let captured = requests.lock().unwrap();
    assert_eq!(
        captured.len(),
        2,
        "one original attempt and one retry: {:?}",
        captured
            .iter()
            .map(|r| r.messages.len())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        captured[1].messages, captured[0].messages,
        "the retry request carries the turn-start conversation unchanged"
    );
    assert!(
        !captured[1]
            .messages
            .iter()
            .any(|message| message.content.contains("The answ")),
        "the dropped attempt's partial text never entered the conversation"
    );
}
