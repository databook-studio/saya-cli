#[tokio::test]
async fn truncation_is_never_retried_and_emits_no_reset() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = TruncatingProvider {
        requests: requests.clone(),
    };
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
    };
    // O1 property 5 is per-turn: `receive` never retries a truncation. The
    // loop-level continuation budget is disabled here so this test pins the
    // single-attempt receive behavior, not the continuation policy.
    let limits = AgentLimits {
        max_continuations: Some(0),
        ..AgentLimits::default()
    };
    let error = run_agent_with_sink(
        &provider,
        &MockTools {
            calls: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        limits,
        &AllowReadOnlyApproval,
        &sink,
        CancellationToken::new(),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(
            error,
            AgentError::Provider(saya_agent::ProviderError::OutputTruncated { .. })
        ),
        "the typed truncation error reaches the caller: {error:?}"
    );
    assert_eq!(
        requests.lock().unwrap().len(),
        1,
        "a truncation is deterministic: exactly one attempt"
    );
    let seen = events.lock().unwrap().clone();
    assert!(
        !seen
            .iter()
            .any(|event| matches!(event, AgentEvent::TurnReset)),
        "no reset means no 'interrupted — retrying' line: {seen:?}"
    );
}
