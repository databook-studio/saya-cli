/// A turn that ran a parallel batch of tool calls must still have its
/// conversation trimmed to the byte budget. Before the fix the batch path
/// returned `true` and the loop `continue`d past `trim_to_budget`, so a run
/// using parallel tool calls never enforced the budget at all — every later
/// turn shipped an ever-growing payload. This drives the batch path (two
/// auto-runnable `schema_discovery` calls in one assistant message) with results
/// that together exceed the budget, and asserts every provider request fits.
#[tokio::test]
async fn batch_turn_enforces_the_byte_budget() {
    struct BigBatchTools;
    #[async_trait::async_trait]
    impl ToolExecutor for BigBatchTools {
        async fn execute(
            &self,
            _: &str,
            _: serde_json::Value,
        ) -> Result<serde_json::Value, ToolError> {
            Ok(serde_json::json!({ "rows": vec!["x".repeat(6_000)] }))
        }
    }
    struct CapturingBatchProvider {
        requests: Arc<Mutex<Vec<ChatRequest>>>,
        turn: Mutex<usize>,
    }
    #[async_trait]
    impl ChatProvider for CapturingBatchProvider {
        fn name(&self) -> &str {
            "capture-batch"
        }
        async fn complete(
            &self,
            request: ChatRequest,
        ) -> Result<ChatResponse, saya_agent::ProviderError> {
            self.requests.lock().unwrap().push(request);
            let turn = {
                let mut t = self.turn.lock().unwrap();
                let was = *t;
                *t += 1;
                was
            };
            if turn == 0 {
                return Ok(ChatResponse::new(ChatMessage {
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
                }));
            }
            Ok(ChatResponse::new(ChatMessage::text("assistant", "done")))
        }
    }

    const BUDGET: usize = 8_192;
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = CapturingBatchProvider {
        requests: requests.clone(),
        turn: Mutex::new(0),
    };
    let output = run_agent(
        &provider,
        &BigBatchTools,
        request(),
        definitions(),
        AgentLimits {
            context_byte_budget: BUDGET,
            ..AgentLimits::default()
        },
        &AllowReadOnlyApproval,
    )
    .await
    .expect("a batch turn trims rather than aborting");
    assert_eq!(output.answer, "done");
    for request in requests.lock().unwrap().iter() {
        let total = request
            .messages
            .iter()
            .map(|message| {
                message.content.len() + message.role.len() + message_size_overhead(message)
            })
            .sum::<usize>();
        assert!(
            total <= BUDGET,
            "a batch turn must stay within the {BUDGET}-byte budget; saw {total}"
        );
    }
}
