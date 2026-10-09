/// An HTTP establishment failure has already had provider-layer retry policy.
/// The outer receiver must not replay an invalid request or execute effects.
#[tokio::test]
async fn stream_establishment_failure_is_not_retried() {
    let provider_calls = Arc::new(Mutex::new(0));
    let tool_calls = Arc::new(Mutex::new(0));
    let events = Arc::new(Mutex::new(Vec::new()));
    let metrics = Arc::new(Mutex::new(Vec::new()));
    let error = run_agent_with_sink(
        &EstablishmentFailure {
            calls: provider_calls.clone(),
        },
        &NoTools {
            calls: tool_calls.clone(),
        },
        request(),
        Vec::new(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &CaptureSink {
            events: events.clone(),
            metrics: metrics.clone(),
        },
        CancellationToken::new(),
    )
    .await
    .expect_err("the refusal is terminal");
    assert!(matches!(
        error,
        AgentError::Provider(ProviderError::Request(_))
    ));
    assert_eq!(*provider_calls.lock().unwrap(), 1, "no outer receive retry");
    assert_eq!(*tool_calls.lock().unwrap(), 0, "no effects before a reply");
    let events = events.lock().unwrap();
    assert!(
        events
            .iter()
            .all(|event| !matches!(event, AgentEvent::TurnReset)),
        "a non-retry has no reset: {events:?}"
    );
    assert!(
        events.iter().any(|event| matches!(
            event,
            AgentEvent::ProviderRecovery {
                phase: ProviderRecoveryPhase::NotRetried,
                reason: ProviderRecoveryReason::ProviderFailure,
                attempt: 1,
                limit: 4,
            }
        )),
        "the receipt is typed but never derives a reason from provider prose: {events:?}"
    );
    let metrics = metrics.lock().unwrap();
    assert!(metrics.iter().any(|metric| matches!(
        metric,
        AgentMetric::Phase {
            phase: AgentMetricPhase::ProviderEstablishment,
            outcome: AgentMetricOutcome::Failed,
            ..
        }
    )));
    for phase in [
        AgentMetricPhase::ProviderReadAwait,
        AgentMetricPhase::FirstProviderEvent,
    ] {
        assert!(metrics.iter().any(|metric| matches!(
            metric,
            AgentMetric::Phase { phase: seen, elapsed_ns: 0, outcome: AgentMetricOutcome::Unavailable }
                if *seen == phase
        )));
    }
}
