#[tokio::test]
async fn a_completed_but_empty_response_errors_after_one_attempt() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = EmptyCompletionProvider {
        requests: requests.clone(),
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
        &NoopSink,
        CancellationToken::new(),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(
            error,
            AgentError::Provider(saya_agent::ProviderError::InvalidResponse)
        ),
        "the existing error path applies: {error:?}"
    );
    assert_eq!(
        requests.lock().unwrap().len(),
        1,
        "a completed-but-empty response is not retried"
    );
}
