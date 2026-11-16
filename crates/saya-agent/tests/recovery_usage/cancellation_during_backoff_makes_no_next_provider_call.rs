#[tokio::test]
async fn cancellation_during_backoff_makes_no_next_provider_call() {
    let calls = Arc::new(Mutex::new(0));
    let cancellation = CancellationToken::new();
    let metrics = Arc::new(Mutex::new(Vec::new()));
    let error = run_agent_with_sink(
        &DroppingProvider {
            calls: calls.clone(),
        },
        &NoTools {
            calls: Arc::new(Mutex::new(0)),
        },
        request(),
        Vec::new(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &CancelOnRetry {
            cancellation: cancellation.clone(),
            metrics: metrics.clone(),
        },
        cancellation,
    )
    .await
    .expect_err("cancellation abandons the local backoff");
    assert!(matches!(error, AgentError::Cancelled));
    assert_eq!(*calls.lock().unwrap(), 1);
    let metrics = metrics.lock().unwrap();
    assert!(metrics.iter().any(|metric| matches!(
        metric,
        AgentMetric::Phase {
            phase: AgentMetricPhase::RetryBackoff,
            outcome: AgentMetricOutcome::Cancelled,
            ..
        }
    )));
    assert!(!metrics.contains(&AgentMetric::Count(AgentMetricCount::RetryAttemptStarted)));
}
