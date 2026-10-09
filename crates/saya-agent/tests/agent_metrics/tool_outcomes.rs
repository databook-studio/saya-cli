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
