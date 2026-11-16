#[tokio::test]
async fn unknown_failed_usage_is_absent_but_reported_zero_is_a_receipt() {
    for first_usage in [None, Some(TokenUsage::new(0, 0))] {
        let events = Arc::new(Mutex::new(Vec::new()));
        run_agent_with_sink(
            &OptionalUsageThenSuccess {
                attempts: Mutex::new(0),
                first_usage,
            },
            &NoTools {
                calls: Arc::new(Mutex::new(0)),
            },
            request(),
            Vec::new(),
            AgentLimits::default(),
            &AllowReadOnlyApproval,
            &RecordingSink(events.clone()),
            CancellationToken::new(),
        )
        .await
        .expect("the retry answers");
        let receipt_count = events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| matches!(event, AgentEvent::FailedAttemptUsage { .. }))
            .count();
        assert_eq!(receipt_count, usize::from(first_usage.is_some()));
    }
}
