struct EntryCountingProvider(AtomicUsize);

#[async_trait]
impl ChatProvider for EntryCountingProvider {
    fn name(&self) -> &str {
        "entry-counting-metric-test"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("the run receives through stream")
    }

    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(Box::pin(futures_util::stream::iter([
            Ok(ProviderEvent::TextDelta("answer".into())),
            Ok(ProviderEvent::Done),
        ])))
    }
}

struct CancelOnTurnStarted {
    cancellation: CancellationToken,
    metrics: Mutex<Vec<AgentMetric>>,
}

#[async_trait]
impl AgentEventSink for CancelOnTurnStarted {
    async fn emit(&self, event: AgentEvent) {
        if matches!(event, AgentEvent::TurnStarted) {
            self.cancellation.cancel();
        }
    }

    async fn metric(&self, metric: AgentMetric) {
        self.metrics.lock().unwrap().push(metric);
    }
}

#[tokio::test]
async fn turn_started_cancellation_prevents_provider_entry() {
    let provider = EntryCountingProvider(AtomicUsize::new(0));
    let mut terminal_metrics = Vec::new();
    let mut results = Vec::new();
    // The baseline has an unbiased select with both cancellation and source ready.
    for _ in 0..32 {
        let cancellation = CancellationToken::new();
        let sink = CancelOnTurnStarted {
            cancellation: cancellation.clone(),
            metrics: Mutex::new(Vec::new()),
        };
        results.push(
            run_agent_with_sink(
                &provider,
                &NoTools,
                request("question"),
                Vec::new(),
                AgentLimits::default(),
                &AllowReadOnlyApproval,
                &sink,
                cancellation,
            )
            .await,
        );
        terminal_metrics.push(sink.metrics.into_inner().unwrap());
    }
    assert_eq!(provider.0.load(Ordering::SeqCst), 0);
    assert!(
        results
            .iter()
            .all(|result| matches!(result, Err(AgentError::Cancelled)))
    );
    let metrics = terminal_metrics.into_iter().flatten().collect::<Vec<_>>();
    let unavailable = |phase| AgentMetric::Phase {
        phase,
        elapsed_ns: 0,
        outcome: AgentMetricOutcome::Unavailable,
    };
    for metric in [
        unavailable(AgentMetricPhase::ProviderEstablishment),
        unavailable(AgentMetricPhase::ProviderReadAwait),
        unavailable(AgentMetricPhase::FirstProviderEvent),
    ] {
        assert_eq!(
            metrics.iter().filter(|sample| **sample == metric).count(),
            32
        );
    }
    assert_eq!(
        metrics
            .iter()
            .filter(|sample| matches!(
                sample,
                AgentMetric::Phase {
                    phase: AgentMetricPhase::LocalAttempt,
                    outcome: AgentMetricOutcome::Cancelled,
                    ..
                }
            ))
            .count(),
        32
    );
    assert_eq!(
        metrics
            .iter()
            .filter(|sample| **sample
                == AgentMetric::UsageCoverage {
                    attempt_outcome: AgentAttemptOutcome::Cancelled,
                    reported: false,
                })
            .count(),
        32
    );
}
