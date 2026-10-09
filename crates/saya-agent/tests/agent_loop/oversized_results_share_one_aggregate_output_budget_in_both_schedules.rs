/// Serial and explicitly concurrent calls share the same aggregate result
/// budget. A delayed first completion exercises ordered concurrent buffering;
/// a success completion containing "failed" proves status remains typed.
#[tokio::test]
async fn oversized_results_share_one_aggregate_output_budget_in_both_schedules() {
    use std::time::Duration;

    const CALLS: usize = 8;
    const AGGREGATE_CAP: usize = 256 * 1024;

    struct SlowFirstTools;
    #[async_trait::async_trait]
    impl ToolExecutor for SlowFirstTools {
        async fn execute(
            &self,
            _: &str,
            arguments: serde_json::Value,
        ) -> Result<serde_json::Value, ToolError> {
            let which = arguments["which"].as_u64().expect("test supplies an index");
            if which == 0 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            if which == (CALLS - 1) as u64 {
                return Err(ToolError::QueryFailedDetail("actual failure".into()));
            }
            Ok(serde_json::json!({"rows": ["x".repeat(80_000)]}))
        }
    }

    struct Provider {
        requests: Arc<Mutex<Vec<ChatRequest>>>,
        turn: Mutex<usize>,
    }
    #[async_trait]
    impl ChatProvider for Provider {
        fn name(&self) -> &str {
            "aggregate-capture"
        }

        async fn complete(
            &self,
            request: ChatRequest,
        ) -> Result<ChatResponse, saya_agent::ProviderError> {
            self.requests.lock().unwrap().push(request);
            let mut turn = self.turn.lock().unwrap();
            if *turn == 0 {
                *turn += 1;
                return Ok(ChatResponse::new(ChatMessage {
                    role: "assistant".into(),
                    content: String::new(),
                    tool_calls: (0..CALLS)
                        .map(|which| ToolCall {
                            id: format!("call-{which}"),
                            name: "schema_discovery".into(),
                            arguments: serde_json::json!({"which": which}),
                        })
                        .collect(),
                    tool_call_id: None,
                }));
            }
            Ok(ChatResponse::new(ChatMessage::text("assistant", "done")))
        }
    }

    for concurrency in [ToolConcurrency::Serial, ToolConcurrency::Concurrent] {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let mut tool_definitions = definitions();
        tool_definitions[2].concurrency = concurrency;
        tool_definitions[2].completion = Some("schema failed text".into());
        let output = run_agent(
            &Provider {
                requests: requests.clone(),
                turn: Mutex::new(0),
            },
            &SlowFirstTools,
            request(),
            tool_definitions,
            AgentLimits {
                context_byte_budget: 1024 * 1024,
                ..AgentLimits::default()
            },
            &AllowReadOnlyApproval,
        )
        .await
        .expect("bounded results complete");
        assert_eq!(output.answer, "done");
        assert_eq!(
            output
                .tool_metadata
                .iter()
                .map(|metadata| metadata.status.as_str())
                .collect::<Vec<_>>(),
            ["completed"; CALLS - 1]
                .into_iter()
                .chain(["failed"])
                .collect::<Vec<_>>(),
            "a success summary containing 'failed' remains completed"
        );
        let request = requests.lock().unwrap()[1].clone();
        let results: Vec<_> = request
            .messages
            .iter()
            .filter(|message| message.role == "tool")
            .collect();
        assert_eq!(results.len(), CALLS, "one result per accepted call");
        let expected_ids: Vec<_> = (0..CALLS).map(|which| format!("call-{which}")).collect();
        assert_eq!(
            results
                .iter()
                .map(|message| message.tool_call_id.as_deref())
                .collect::<Vec<_>>(),
            expected_ids
                .iter()
                .map(String::as_str)
                .map(Some)
                .collect::<Vec<_>>(),
            "results stay in call order"
        );
        assert!(
            results[0].content.contains("\"rows\":[\"xxx"),
            "the bounded preview retains useful result content"
        );
        assert!(
            results
                .iter()
                .map(|message| message.content.len())
                .sum::<usize>()
                <= AGGREGATE_CAP,
            "all model-facing tool content must fit the aggregate cap"
        );
    }
}
