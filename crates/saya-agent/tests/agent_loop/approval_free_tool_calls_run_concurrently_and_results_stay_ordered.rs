/// Proves approval-free tool calls in one assistant message overlap: each
/// executor entry waits at a 2-party barrier, so sequential execution would
/// deadlock (surfacing as a timeout) instead of passing.
#[tokio::test]
async fn approval_free_tool_calls_run_concurrently_and_results_stay_ordered() {
    use std::time::Duration;
    use tokio::sync::Barrier;

    #[derive(Default)]
    struct Diagnostics(Mutex<Vec<saya_agent::ToolDiagnostic>>);
    #[async_trait::async_trait]
    impl AgentEventSink for Diagnostics {
        async fn emit(&self, _: AgentEvent) {}
        async fn observe_tool(&self, diagnostic: saya_agent::ToolDiagnostic) {
            self.0.lock().unwrap().push(diagnostic);
        }
    }
    struct BarrierTools {
        barrier: Arc<Barrier>,
        seen: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait::async_trait]
    impl ToolExecutor for BarrierTools {
        async fn execute(
            &self,
            name: &str,
            arguments: serde_json::Value,
        ) -> Result<serde_json::Value, ToolError> {
            self.barrier.wait().await;
            self.seen.lock().unwrap().push(name.into());
            if arguments["which"] == 1 {
                Ok(serde_json::json!({"outcome":"refused", "reason":"PRIVATE_REFUSAL"}))
            } else {
                Err(ToolError::Runner("PRIVATE_FAILURE".into()))
            }
        }
    }

    let provider = MockProvider {
        responses: Mutex::new(vec![
            ChatResponse::new(ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![
                    ToolCall {
                        id: "call-a".into(),
                        name: "schema_discovery".into(),
                        arguments: serde_json::json!({"which": 1}),
                    },
                    ToolCall {
                        id: "call-b".into(),
                        name: "schema_discovery".into(),
                        arguments: serde_json::json!({"which": 2}),
                    },
                ],
                tool_call_id: None,
                images: Vec::new(),
                            reasoning: None,
                            reasoning_signature: None,
            }),
            ChatResponse::new(ChatMessage::text("assistant", "parallel done")),
        ]),
    };
    let tools = BarrierTools {
        barrier: Arc::new(Barrier::new(2)),
        seen: Arc::new(Mutex::new(Vec::new())),
    };
    let sink = Diagnostics::default();
    let run = tokio::time::timeout(
        Duration::from_secs(5),
        run_agent_with_sink(
            &provider,
            &tools,
            request(),
            definitions(),
            AgentLimits::default(),
            &AllowReadOnlyApproval,
            &sink,
            CancellationToken::new(),
        ),
    )
    .await
    .expect("calls must overlap; sequential execution deadlocks at the barrier");
    let output = run.unwrap();
    assert_eq!(output.answer, "parallel done");
    assert_eq!(
        &*tools.seen.lock().unwrap(),
        &["schema_discovery", "schema_discovery"]
    );
    use saya_agent::ToolDiagnosticStatus::{Failed, Refused, Requested};
    let records = sink.0.lock().unwrap();
    assert_eq!(
        records
            .iter()
            .map(|r| r.status())
            .collect::<Vec<_>>()
            .iter()
            .filter(|s| **s == Requested)
            .count(),
        2
    );
    assert_eq!(records.iter().filter(|r| r.status() == Refused).count(), 1);
    assert_eq!(records.iter().filter(|r| r.status() == Failed).count(), 1);
    assert!(records.iter().all(|r| r.tool() == "schema_discovery"));
    // Each call's terminal row carries that call's own transcript — the id the
    // provider gave it, the arguments the model sent, and the result the
    // executor returned — while the dispatch rows beside them stay metadata
    // only. The batch overlaps, so the pairing is by call id, never by order.
    let transcripts = records
        .iter()
        .filter_map(|record| record.transcript())
        .collect::<Vec<_>>();
    assert_eq!(transcripts.len(), 2, "one transcript per executed call");
    assert!(
        transcripts
            .iter()
            .all(|transcript| transcript.arguments() == r#"{"which":1}"#
                || transcript.arguments() == r#"{"which":2}"#),
        "{transcripts:?}"
    );
    assert!(
        transcripts
            .iter()
            .any(|transcript| transcript.call_id() == "call-a"
                && transcript.result().is_some_and(|result| result.contains("PRIVATE_REFUSAL"))),
        "the refused call records the result it returned: {transcripts:?}"
    );
    assert!(
        transcripts
            .iter()
            .any(|transcript| transcript.call_id() == "call-b"
                && transcript.result().is_some_and(|result| result.contains("PRIVATE_FAILURE"))),
        "the failed call records the error the model was handed back: {transcripts:?}"
    );
}
