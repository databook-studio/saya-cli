#[tokio::test]
async fn retry_metrics_keep_absent_and_reported_zero_usage_distinct() {
    for (failed_usage, successful_usage, expected_reported) in [
        (None, Some(TokenUsage::new(0, 0)), [false, true]),
        (Some(TokenUsage::new(0, 0)), None, [true, false]),
    ] {
        let metrics = Arc::new(Mutex::new(Vec::new()));
        let output = run_agent_with_sink(
            &OptionalUsageRetry {
                calls: Mutex::new(0),
                failed_usage,
                successful_usage,
            },
            &NoTools {
                calls: Arc::new(Mutex::new(0)),
            },
            request(),
            Vec::new(),
            AgentLimits::default(),
            &AllowReadOnlyApproval,
            &MetricSink(metrics.clone()),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(output.answer, "kept");
        let metrics = metrics.lock().unwrap();
        let coverage = metrics
            .iter()
            .filter_map(|metric| match metric {
                AgentMetric::UsageCoverage {
                    attempt_outcome,
                    reported,
                } => Some((*attempt_outcome, *reported)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            coverage,
            vec![
                (AgentAttemptOutcome::Failed, expected_reported[0]),
                (AgentAttemptOutcome::Succeeded, expected_reported[1]),
            ]
        );
        assert_eq!(
            metrics
                .iter()
                .filter(
                    |metric| **metric == AgentMetric::Count(AgentMetricCount::RetryAttemptStarted)
                )
                .count(),
            1
        );
        assert!(metrics.iter().any(|metric| matches!(
            metric,
            AgentMetric::Phase {
                phase: AgentMetricPhase::RetryBackoff,
                outcome: AgentMetricOutcome::Succeeded,
                ..
            }
        )));
    }
}
