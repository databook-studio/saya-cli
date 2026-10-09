#[tokio::test]
async fn genuine_transport_failure_still_retries_on_the_existing_schedule() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = AlwaysFailingProvider {
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
            AgentError::Provider(saya_agent::ProviderError::Request(_))
        ),
        "the transport error falls through after the schedule: {error:?}"
    );
    assert_eq!(
        requests.lock().unwrap().len(),
        4,
        "one attempt plus the three scheduled retries"
    );
    assert_eq!(
        events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| matches!(event, AgentEvent::TurnReset))
            .count(),
        3,
        "one reset per retry"
    );
}
