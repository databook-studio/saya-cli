/// A loop with no turn ceiling must keep bounding memory: `trim_to_budget`
/// still caps the conversation at `context_byte_budget` every turn, so a long
/// run cannot exhaust RAM. A provider that runs many turns — each returning a
/// result far larger than the budget — must complete, and every provider
/// request must fit the budget.
#[tokio::test]
async fn unbounded_loop_trims_context_to_budget() {
    struct BigTools;
    #[async_trait::async_trait]
    impl ToolExecutor for BigTools {
        async fn execute(
            &self,
            _: &str,
            _: serde_json::Value,
        ) -> Result<serde_json::Value, ToolError> {
            Ok(serde_json::json!({ "rows": vec!["x".repeat(8_192)] }))
        }
    }
    struct LongRunProvider {
        requests: Arc<Mutex<Vec<ChatRequest>>>,
        turns: usize,
    }
    #[async_trait]
    impl ChatProvider for LongRunProvider {
        fn name(&self) -> &str {
            "long"
        }
        async fn complete(
            &self,
            request: ChatRequest,
        ) -> Result<ChatResponse, saya_agent::ProviderError> {
            let turn = {
                let mut requests = self.requests.lock().unwrap();
                requests.push(request);
                requests.len()
            };
            if turn <= self.turns {
                Ok(ChatResponse::new(ChatMessage {
                    role: "assistant".into(),
                    content: String::new(),
                    tool_calls: vec![ToolCall {
                        id: format!("c{turn}"),
                        name: "schema_discovery".into(),
                        arguments: serde_json::json!({}),
                    }],
                    tool_call_id: None,
                }))
            } else {
                Ok(ChatResponse::new(ChatMessage::text("assistant", "done")))
            }
        }
    }
    const TURNS: usize = 30;
    const BUDGET: usize = 4_096;
    let requests = Arc::new(Mutex::new(Vec::new()));
    let output = run_agent(
        &LongRunProvider {
            requests: requests.clone(),
            turns: TURNS,
        },
        &BigTools,
        request(),
        definitions(),
        AgentLimits {
            context_byte_budget: BUDGET,
            ..AgentLimits::default()
        },
        &AllowReadOnlyApproval,
    )
    .await
    .expect("an unbounded run completes by trimming, not by a turn ceiling");
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
            "each turn must stay within the {BUDGET}-byte budget; saw {total}"
        );
    }
}
