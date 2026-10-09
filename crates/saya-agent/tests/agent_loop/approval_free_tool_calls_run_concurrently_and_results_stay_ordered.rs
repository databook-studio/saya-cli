/// Proves approval-free tool calls in one assistant message overlap: each
/// executor entry waits at a 2-party barrier, so sequential execution would
/// deadlock (surfacing as a timeout) instead of passing.
#[tokio::test]
async fn approval_free_tool_calls_run_concurrently_and_results_stay_ordered() {
    use std::time::Duration;
    use tokio::sync::Barrier;

    struct BarrierTools {
        barrier: Arc<Barrier>,
        seen: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait::async_trait]
    impl ToolExecutor for BarrierTools {
        async fn execute(
            &self,
            name: &str,
            _: serde_json::Value,
        ) -> Result<serde_json::Value, ToolError> {
            self.barrier.wait().await;
            self.seen.lock().unwrap().push(name.into());
            Ok(serde_json::json!({"ok": true}))
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
            }),
            ChatResponse::new(ChatMessage::text("assistant", "parallel done")),
        ]),
    };
    let tools = BarrierTools {
        barrier: Arc::new(Barrier::new(2)),
        seen: Arc::new(Mutex::new(Vec::new())),
    };
    let run = tokio::time::timeout(
        Duration::from_secs(5),
        run_agent(
            &provider,
            &tools,
            request(),
            definitions(),
            AgentLimits::default(),
            &AllowReadOnlyApproval,
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
}
