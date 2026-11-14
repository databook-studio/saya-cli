#[tokio::test]
async fn exhausted_retries_fall_through_to_the_existing_error_path() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = AlwaysDroppingProvider {
        requests: requests.clone(),
    };
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
    };
    let error = run_agent_with_sink(
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
    .unwrap_err();
    assert!(
        matches!(
            error,
            AgentError::Provider(saya_agent::ProviderError::StreamEnded)
        ),
        "the exhausted schedule falls through to the existing error: {error:?}"
    );
    assert_eq!(
        requests.lock().unwrap().len(),
        9,
        "bounded: one attempt plus the eight scheduled retries"
    );
    let seen = events.lock().unwrap().clone();
    assert!(
        !seen
            .iter()
            .any(|event| matches!(event, AgentEvent::Complete)),
        "an exhausted retry run never completes: {seen:?}"
    );
}
