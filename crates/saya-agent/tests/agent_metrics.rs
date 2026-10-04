use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, AgentLimits, AgentMetric, AgentMetricOutcome, AgentMetricPhase,
    AgentMetricUseful, AgentRequest, AllowReadOnlyApproval, CancellationToken, ChatProvider,
    ChatRequest, ChatResponse, ProviderError, ProviderEvent, ProviderStream, TokenUsage, ToolCall,
    ToolConcurrency, ToolDefinition, ToolEffect, ToolError, ToolExecutor, run_agent_with_sink,
};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::oneshot;

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

struct ToolThenAnswer(Mutex<usize>);

#[async_trait]
impl ChatProvider for ToolThenAnswer {
    fn name(&self) -> &str {
        "tool-metric-test"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("the run receives through stream")
    }

    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        let mut calls = self.0.lock().unwrap();
        *calls += 1;
        let events = if *calls == 1 {
            vec![
                Ok(ProviderEvent::ToolCalls(vec![ToolCall {
                    id: "tool-metric-id".into(),
                    name: "safe_read".into(),
                    arguments: serde_json::json!({"private":"PRIVATE-TOOL-ARG-SENTINEL"}),
                }])),
                Ok(ProviderEvent::Done),
            ]
        } else {
            vec![
                Ok(ProviderEvent::TextDelta("finished".into())),
                Ok(ProviderEvent::Done),
            ]
        };
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}

struct SuccessfulTool;

#[async_trait]
impl ToolExecutor for SuccessfulTool {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        Ok(serde_json::json!({
            "row_count":1,
            "columns":["value"],
            "rows":[["PRIVATE-TOOL-RESULT-SENTINEL"]]
        }))
    }
}

struct FailingTool;

#[async_trait]
impl ToolExecutor for FailingTool {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        Err(ToolError::QueryFailedDetail(
            "PRIVATE-TOOL-ERROR-SENTINEL".into(),
        ))
    }
}

struct DenyApproval;

#[async_trait]
impl saya_agent::ApprovalDecider for DenyApproval {
    async fn approve(&self, _: &ToolDefinition, _: &serde_json::Value) -> bool {
        false
    }
}

fn tool_definition(requires_approval: bool) -> ToolDefinition {
    ToolDefinition {
        name: "safe_read".into(),
        description: "bounded read".into(),
        read_only: true,
        parameters: serde_json::json!({"type":"object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval,
            local_state: saya_agent::LocalStateEffect::None,
        },
        concurrency: ToolConcurrency::Serial,
        completion: None,
    }
}

#[tokio::test]
async fn useful_tool_metric_requires_a_successful_executor_result() {
    let sink = MetricSink::default();
    let output = run_agent_with_sink(
        &ToolThenAnswer(Mutex::new(0)),
        &SuccessfulTool,
        request("question"),
        vec![tool_definition(false)],
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &sink,
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(output.answer, "finished");
    assert_eq!(
        sink.metrics
            .lock()
            .unwrap()
            .iter()
            .filter(|metric| **metric == AgentMetric::Useful(AgentMetricUseful::SuccessfulTool))
            .count(),
        1
    );
    let metrics = sink.metrics.lock().unwrap();
    assert!(!format!("{metrics:?}").contains("PRIVATE-TOOL-ARG-SENTINEL"));
    assert!(!format!("{metrics:?}").contains("PRIVATE-TOOL-RESULT-SENTINEL"));
}

#[tokio::test]
async fn denied_and_failed_tools_do_not_emit_useful_metrics_or_dynamic_labels() {
    let failed_sink = MetricSink::default();
    run_agent_with_sink(
        &ToolThenAnswer(Mutex::new(0)),
        &FailingTool,
        request("question"),
        vec![tool_definition(false)],
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &failed_sink,
        CancellationToken::new(),
    )
    .await
    .unwrap();
    {
        let failed_metrics = failed_sink.metrics.lock().unwrap();
        assert!(!failed_metrics.contains(&AgentMetric::Useful(AgentMetricUseful::SuccessfulTool)));
        assert!(!format!("{failed_metrics:?}").contains("PRIVATE-TOOL-ERROR-SENTINEL"));
    }

    let denied_sink = MetricSink::default();
    run_agent_with_sink(
        &ToolThenAnswer(Mutex::new(0)),
        &SuccessfulTool,
        request("question"),
        vec![tool_definition(true)],
        AgentLimits::default(),
        &DenyApproval,
        &denied_sink,
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert!(
        !denied_sink
            .metrics
            .lock()
            .unwrap()
            .contains(&AgentMetric::Useful(AgentMetricUseful::SuccessfulTool))
    );
}

struct ParallelToolProvider(Mutex<usize>);

#[async_trait]
impl ChatProvider for ParallelToolProvider {
    fn name(&self) -> &str {
        "parallel-tool-metric-test"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("the run receives through stream")
    }

    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        let mut calls = self.0.lock().unwrap();
        *calls += 1;
        let events = if *calls == 1 {
            vec![
                Ok(ProviderEvent::ToolCalls(vec![
                    ToolCall {
                        id: "fast-id".into(),
                        name: "fast_read".into(),
                        arguments: serde_json::json!({}),
                    },
                    ToolCall {
                        id: "slow-id".into(),
                        name: "slow_read".into(),
                        arguments: serde_json::json!({}),
                    },
                ])),
                Ok(ProviderEvent::Done),
            ]
        } else {
            vec![
                Ok(ProviderEvent::TextDelta("finished".into())),
                Ok(ProviderEvent::Done),
            ]
        };
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}

struct HeldSlowTool {
    started: Mutex<Option<oneshot::Sender<()>>>,
    release: Mutex<Option<oneshot::Receiver<()>>>,
}

#[async_trait]
impl ToolExecutor for HeldSlowTool {
    async fn execute(
        &self,
        name: &str,
        _: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        if name == "slow_read" {
            if let Some(started) = self.started.lock().unwrap().take() {
                let _ = started.send(());
            }
            let release = { self.release.lock().unwrap().take() };
            if let Some(release) = release {
                let _ = release.await;
            }
        }
        Ok(serde_json::json!({"row_count":1,"columns":["value"]}))
    }
}

fn concurrent_definition(name: &str) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: "bounded read".into(),
        read_only: true,
        parameters: serde_json::json!({"type":"object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: saya_agent::LocalStateEffect::None,
        },
        concurrency: ToolConcurrency::Concurrent,
        completion: None,
    }
}

#[tokio::test]
async fn concurrent_useful_metric_arrives_before_slow_sibling_settles() {
    let (slow_started_tx, slow_started_rx) = oneshot::channel();
    let (slow_release_tx, slow_release_rx) = oneshot::channel();
    let tools = HeldSlowTool {
        started: Mutex::new(Some(slow_started_tx)),
        release: Mutex::new(Some(slow_release_rx)),
    };
    let (metric_tx, mut metric_rx) = tokio::sync::mpsc::unbounded_channel();
    let sink = MetricSink::default();
    *sink.useful_metric.lock().unwrap() = Some(metric_tx);
    let provider = ParallelToolProvider(Mutex::new(0));
    let approval = AllowReadOnlyApproval;
    let run = run_agent_with_sink(
        &provider,
        &tools,
        request("question"),
        vec![
            concurrent_definition("fast_read"),
            concurrent_definition("slow_read"),
        ],
        AgentLimits::default(),
        &approval,
        &sink,
        CancellationToken::new(),
    );
    tokio::pin!(run);
    tokio::select! {
        result = &mut run => panic!("the slow tool should hold the batch: {result:?}"),
        _ = slow_started_rx => {},
    }
    tokio::time::timeout(Duration::from_secs(1), async {
        tokio::select! {
            metric = metric_rx.recv() => metric.expect("a successful fast tool is useful"),
            result = &mut run => panic!("batch settled before releasing slow tool: {result:?}"),
        }
    })
    .await
    .expect("fast result metric is delivered while its sibling remains held");
    slow_release_tx.send(()).unwrap();
    let output = run.await.unwrap();
    assert_eq!(output.answer, "finished");
    assert_eq!(
        sink.metrics
            .lock()
            .unwrap()
            .iter()
            .filter(|metric| **metric == AgentMetric::Useful(AgentMetricUseful::SuccessfulTool))
            .count(),
        2
    );
}
