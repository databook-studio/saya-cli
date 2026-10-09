struct HeldProvider {
    entered: Mutex<Option<oneshot::Sender<()>>>,
    release: Mutex<Option<oneshot::Receiver<()>>>,
}

#[async_trait]
impl ChatProvider for HeldProvider {
    fn name(&self) -> &str {
        "fixed-metric-test"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("the run receives through stream")
    }

    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        if let Some(entered) = self.entered.lock().unwrap().take() {
            let _ = entered.send(());
        }
        let release = { self.release.lock().unwrap().take() };
        if let Some(release) = release {
            let _ = release.await;
        }
        Ok(Box::pin(futures_util::stream::iter([
            Ok(ProviderEvent::TextDelta("   ".into())),
            Ok(ProviderEvent::TextDelta("useful answer".into())),
            Ok(ProviderEvent::Usage(TokenUsage::new(0, 0))),
            Ok(ProviderEvent::Done),
        ])))
    }
}

#[derive(Default)]
struct MetricSink {
    metrics: Mutex<Vec<AgentMetric>>,
    text_entered: Mutex<Option<oneshot::Sender<()>>>,
    release_text: Mutex<Option<oneshot::Receiver<()>>>,
    useful_metric: Mutex<Option<tokio::sync::mpsc::UnboundedSender<()>>>,
}

#[async_trait]
impl AgentEventSink for MetricSink {
    async fn emit(&self, event: AgentEvent) {
        if matches!(&event, AgentEvent::AssistantText { .. }) {
            if let Some(entered) = self.text_entered.lock().unwrap().take() {
                let _ = entered.send(());
            }
            let release = { self.release_text.lock().unwrap().take() };
            if let Some(release) = release {
                let _ = release.await;
            }
        }
    }

    async fn metric(&self, metric: AgentMetric) {
        self.metrics.lock().unwrap().push(metric);
        if metric == AgentMetric::Useful(AgentMetricUseful::SuccessfulTool)
            && let Some(sender) = self.useful_metric.lock().unwrap().as_ref()
        {
            let _ = sender.send(());
        }
    }
}

fn request(prompt: &str) -> AgentRequest {
    AgentRequest {
        prompt: prompt.into(),
        profile_names: Vec::new(),
        model: "metric-test".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

#[derive(Default)]
struct NoTools;

#[async_trait]
impl ToolExecutor for NoTools {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        unreachable!("the answer has no tool call")
    }
}

#[tokio::test]
async fn provider_spans_exclude_display_wait_while_local_attempt_includes_it() {
    let (provider_entered_tx, provider_entered_rx) = oneshot::channel();
    let (provider_release_tx, provider_release_rx) = oneshot::channel();
    let (text_entered_tx, text_entered_rx) = oneshot::channel();
    let (text_release_tx, text_release_rx) = oneshot::channel();
    let provider = HeldProvider {
        entered: Mutex::new(Some(provider_entered_tx)),
        release: Mutex::new(Some(provider_release_rx)),
    };
    let sink = Arc::new(MetricSink::default());
    *sink.text_entered.lock().unwrap() = Some(text_entered_tx);
    *sink.release_text.lock().unwrap() = Some(text_release_rx);
    let approval = AllowReadOnlyApproval;
    let run = run_agent_with_sink(
        &provider,
        &NoTools,
        request("PRIVATE-PROMPT-SENTINEL"),
        Vec::new(),
        AgentLimits::default(),
        &approval,
        sink.as_ref(),
        CancellationToken::new(),
    );
    tokio::pin!(run);
    tokio::select! {
        result = &mut run => panic!("provider returned before its release: {result:?}"),
        _ = provider_entered_rx => {},
    }
    provider_release_tx.send(()).unwrap();
    tokio::select! {
        result = &mut run => panic!("display returned before its release: {result:?}"),
        _ = text_entered_rx => {},
    }
    let display_hold = Instant::now();
    tokio::time::sleep(Duration::from_millis(30)).await;
    text_release_tx.send(()).unwrap();
    let held_ns = display_hold.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
    let output = run.await.unwrap();

    let metrics = sink.metrics.lock().unwrap().clone();
    let phase = |kind| {
        metrics.iter().find_map(|metric| match metric {
            AgentMetric::Phase {
                phase,
                elapsed_ns,
                outcome,
            } if *phase == kind => Some((*elapsed_ns, *outcome)),
            _ => None,
        })
    };
    let (establishment_ns, establishment_outcome) =
        phase(AgentMetricPhase::ProviderEstablishment).unwrap();
    let (local_ns, local_outcome) = phase(AgentMetricPhase::LocalAttempt).unwrap();
    assert_eq!(establishment_outcome, AgentMetricOutcome::Succeeded);
    assert_eq!(local_outcome, AgentMetricOutcome::Succeeded);
    assert!(
        establishment_ns < held_ns,
        "source wait ended before display hold"
    );
    assert!(local_ns >= establishment_ns.saturating_add(held_ns.saturating_sub(2_000_000)));
    assert!(matches!(
        phase(AgentMetricPhase::FirstProviderEvent),
        Some((_, AgentMetricOutcome::Succeeded))
    ));
    assert!(matches!(
        phase(AgentMetricPhase::ProviderReadAwait),
        Some((_, AgentMetricOutcome::Succeeded))
    ));
    assert!(metrics.contains(&AgentMetric::Useful(AgentMetricUseful::NonWhitespaceText)));
    assert_eq!(
        metrics
            .iter()
            .filter(|metric| **metric == AgentMetric::Useful(AgentMetricUseful::NonWhitespaceText))
            .count(),
        1,
        "whitespace-only source items are not useful"
    );
    assert!(metrics.contains(&AgentMetric::UsageCoverage {
        attempt_outcome: saya_agent::AgentAttemptOutcome::Succeeded,
        reported: true,
    }));
    assert!(output.answer.contains("useful answer"));
    assert!(!format!("{metrics:?}").contains("PRIVATE-PROMPT-SENTINEL"));
    assert!(!format!("{output:?}").contains("UsageCoverage"));
    assert!(!output.events.iter().any(|event| matches!(event, AgentEvent::AssistantText { text } if text == "PRIVATE-PROMPT-SENTINEL")));
}
