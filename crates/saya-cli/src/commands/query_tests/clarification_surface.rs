/// One scripted run, both surfaces observed: the headless mapping exits 6,
/// the TUI block carries the same question, and the structured event rides
/// the NDJSON stream.
#[tokio::test]
async fn ambiguous_metric_requires_input_across_surfaces() {
    let provider = ScriptedProvider {
        turns: Mutex::new(vec![clarification_turn()]),
    };
    // The real definitions (the tool advertised) and a no-connector executor:
    // the ask needs no database.
    let definitions = DatabaseTools::definitions(false, false, false, false, false);
    let executor = DatabaseTools::new(None, 10, false);
    let output = run_agent_with_sink(
        &provider,
        &executor,
        agent_request(),
        definitions,
        saya_agent::AgentLimits::default(),
        &AllowReadOnlyApproval,
        &saya_agent::NoopEventSink,
        CancellationToken::new(),
    )
    .await
    .expect("the turn ends on the clarification, never in an error");

    // Headless surface: the run ended on a clarification, so `saya ask`
    // returns the paused exit code, not 0 and not a failure.
    assert_eq!(
        ask_exit_code(&output),
        6,
        "the paused-for-input class is exit 6: {:?}",
        output.events
    );

    // The TUI surface: the transcript's Question block carries the same
    // question. The tui subtree's `apply_event` is private to it, so the
    // block is observed through the shared shaper both adapters render from —
    // the stream_events tests pin that `apply_event` pushes exactly this text.
    let options: Vec<String> = OPTIONS.iter().map(|option| option.to_string()).collect();
    let block_text = clarification_text(QUESTION, &options);
    assert!(
        block_text.contains(QUESTION),
        "the TUI block carries the question: {block_text:?}"
    );
    assert!(
        block_text.contains("1. sessions in the last 30 days")
            && block_text.contains("2. purchases in the last 90 days"),
        "the TUI block numbers the options: {block_text:?}"
    );

    // And the headless NDJSON stream carries the structured event.
    let event = output
        .events
        .iter()
        .find(|event| matches!(event, AgentEvent::ClarificationNeeded { .. }))
        .expect("the clarification event is on the run");
    let terminal = terminal_event(event.clone()).expect("the event renders headlessly");
    let rendered = render_event(&terminal, RenderFormat::Ndjson);
    assert!(
        rendered
            .stdout
            .contains(r#""event":"clarification_needed""#),
        "the ndjson stream tags it: {:?}",
        rendered.stdout
    );
}

/// A run that never asked exits 0 — the mapping must not leak the paused
/// class into ordinary completions.
#[tokio::test]
async fn an_ordinary_completion_exits_zero() {
    let provider = ScriptedProvider {
        turns: Mutex::new(vec![ChatMessage::text("assistant", "42 active users")]),
    };
    let definitions = DatabaseTools::definitions(false, false, false, false, false);
    let executor = DatabaseTools::new(None, 10, false);
    let output = run_agent_with_sink(
        &provider,
        &executor,
        agent_request(),
        definitions,
        saya_agent::AgentLimits::default(),
        &AllowReadOnlyApproval,
        &saya_agent::NoopEventSink,
        CancellationToken::new(),
    )
    .await
    .expect("an ordinary turn completes");
    assert_eq!(ask_exit_code(&output), 0);
    assert_eq!(output.answer, "42 active users");
}
