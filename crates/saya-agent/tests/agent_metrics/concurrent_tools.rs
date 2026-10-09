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
