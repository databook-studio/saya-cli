/// A provider failure on the salvage call itself must not lose the work
/// already done: the run's turns executed tools, gathered events, and billed
/// usage before the cliff, and propagating the error would discard all of it.
/// The run therefore degrades to a structured truncated result — truncated
/// flag set, executed work preserved, and no answer invented — instead of
/// erroring.
#[tokio::test]
async fn salvage_failure_degrades_to_a_truncated_result_keeping_the_work_done() {
    struct FailOnSalvage {
        turn: Mutex<usize>,
    }
    #[async_trait]
    impl ChatProvider for FailOnSalvage {
        fn name(&self) -> &str {
            "fail-salvage"
        }
        async fn complete(
            &self,
            _: ChatRequest,
        ) -> Result<ChatResponse, saya_agent::ProviderError> {
            let turn = {
                let mut t = self.turn.lock().unwrap();
                let was = *t;
                *t += 1;
                was
            };
            if turn == 0 {
                Ok(ChatResponse::new(ChatMessage {
                    role: "assistant".into(),
                    content: String::new(),
                    tool_calls: vec![ToolCall {
                        id: "c1".into(),
                        name: "schema_discovery".into(),
                        arguments: serde_json::json!({}),
                    }],
                    tool_call_id: None,
                }))
            } else {
                Err(saya_agent::ProviderError::Request(
                    "salvage call failed".into(),
                ))
            }
        }
    }
    let calls = Arc::new(Mutex::new(Vec::new()));
    let output = run_agent(
        &FailOnSalvage {
            turn: Mutex::new(0),
        },
        &MockTools {
            calls: calls.clone(),
        },
        request(),
        definitions(),
        AgentLimits {
            max_turns: Some(1),
            ..AgentLimits::default()
        },
        &AllowReadOnlyApproval,
    )
    .await
    .expect("a failed salvage call must not discard the work already done");
    assert!(
        output.truncated,
        "a degraded salvage run must be marked truncated, got {output:?}"
    );
    assert!(
        output.answer.is_empty(),
        "the run must not invent an answer when the salvage call fails"
    );
    assert_eq!(
        &*calls.lock().unwrap(),
        &["schema_discovery"],
        "the turn's executed work must be preserved, not lost"
    );
}
