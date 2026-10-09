/// A loop with no turn ceiling must still end promptly when the cancellation
/// token fires — Ctrl+C has to work when nothing else will stop a runaway
/// model. A provider that never stops calling tools is driven under the
/// default limits and a sink that cancels after many tool requests. The run
/// must return `Cancelled`, and it must have run well past the old fixed turn
/// ceiling to prove it was the cancellation, not a budget, that stopped it.
#[tokio::test]
async fn unbounded_loop_stops_on_cancellation() {
    use std::time::Duration;
    struct ForeverTools;
    #[async_trait::async_trait]
    impl ToolExecutor for ForeverTools {
        async fn execute(
            &self,
            _: &str,
            _: serde_json::Value,
        ) -> Result<serde_json::Value, ToolError> {
            Ok(serde_json::json!({"rows": 1}))
        }
    }
    struct LoopingProvider;
    #[async_trait]
    impl ChatProvider for LoopingProvider {
        fn name(&self) -> &str {
            "loop"
        }
        async fn complete(
            &self,
            _: ChatRequest,
        ) -> Result<ChatResponse, saya_agent::ProviderError> {
            Ok(ChatResponse::new(ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: "call".into(),
                    name: "schema_discovery".into(),
                    arguments: serde_json::json!({}),
                }],
                tool_call_id: None,
            }))
        }
    }
    struct CancelAfter {
        token: CancellationToken,
        seen: Mutex<usize>,
        at: usize,
    }
    #[async_trait]
    impl AgentEventSink for CancelAfter {
        async fn emit(&self, event: AgentEvent) {
            if matches!(event, AgentEvent::ToolRequested { .. }) {
                let mut count = self.seen.lock().unwrap();
                *count += 1;
                if *count == self.at {
                    self.token.cancel();
                }
            }
        }
    }
    let token = CancellationToken::new();
    let sink = CancelAfter {
        token: token.clone(),
        seen: Mutex::new(0),
        at: 20,
    };
    let run = run_agent_with_sink(
        &LoopingProvider,
        &ForeverTools,
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &sink,
        token,
    );
    let error = tokio::time::timeout(Duration::from_secs(5), run)
        .await
        .expect("an unbounded loop must still end on cancellation")
        .unwrap_err();
    assert!(
        matches!(error, AgentError::Cancelled),
        "unbounded loop must end on cancellation, got {error:?}"
    );
}
