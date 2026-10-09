/// No `TurnReset` is emitted on any continuation path: a continuation is not
/// a retry of the same request, so it must not borrow the retry wording.
#[tokio::test]
async fn continuation_emits_no_turn_reset() {
    // The recovery path: one truncation, then a cooperative turn.
    let requests = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    run_agent_with_sink(
        &continuation_script(&requests, false, "partial", Vec::new()),
        &MockTools {
            calls: Arc::new(Mutex::new(Vec::new())),
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
    .await
    .expect("the run recovers");
    // The exhausted path: truncations until the continuation budget runs out.
    let exhausted_requests = Arc::new(Mutex::new(Vec::new()));
    let exhausted_events = Arc::new(Mutex::new(Vec::new()));
    run_agent_with_sink(
        &continuation_script(&exhausted_requests, true, "partial", Vec::new()),
        &MockTools {
            calls: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        AgentLimits {
            max_continuations: Some(3),
            ..AgentLimits::default()
        },
        &AllowReadOnlyApproval,
        &RecordingSink {
            events: exhausted_events.clone(),
        },
        CancellationToken::new(),
    )
    .await
    .unwrap_err();
    for (seen, path) in [
        (events.lock().unwrap().clone(), "recovery"),
        (exhausted_events.lock().unwrap().clone(), "exhausted"),
    ] {
        assert!(
            !seen
                .iter()
                .any(|event| matches!(event, AgentEvent::TurnReset)),
            "{path}: a continuation is not a retry, so no reset may be emitted: {seen:?}"
        );
    }
}
