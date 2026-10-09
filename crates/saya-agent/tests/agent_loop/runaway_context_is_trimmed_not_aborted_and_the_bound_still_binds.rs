/// The intra-loop context bound still binds, but it no longer aborts the run
/// Runaway context is trimmed to fit rather than surfacing an
/// opaque `Limit("context bytes")` after the query already ran. Each provider
/// turn is issued a conversation assembled under the byte budget; the run
/// completes instead of dying on the first oversized turn.
#[tokio::test]
async fn runaway_context_is_trimmed_not_aborted_and_the_bound_still_binds() {
    struct CapturingProvider {
        requests: Arc<Mutex<Vec<ChatRequest>>>,
        turn: Mutex<usize>,
    }
    #[async_trait]
    impl ChatProvider for CapturingProvider {
        fn name(&self) -> &str {
            "capture"
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
            if turn < 4 {
                return Ok(ChatResponse::new(ChatMessage {
                    role: "assistant".into(),
                    content: String::new(),
                    tool_calls: vec![ToolCall {
                        id: format!("call-{turn}"),
                        name: "schema_discovery".into(),
                        arguments: serde_json::json!({"padding": "x".repeat(2_048)}),
                    }],
                    tool_call_id: None,
                }));
            }
            Ok(ChatResponse::new(ChatMessage::text("assistant", "done")))
        }
    }

    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = CapturingProvider {
        requests: requests.clone(),
        turn: Mutex::new(0),
    };
    let output = run_agent(
        &provider,
        &MockTools {
            calls: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        AgentLimits {
            max_turns: Some(8),
            max_tool_calls: Some(64),
            max_continuations: None,
            permit_candidate_writes: false,
            context_byte_budget: 4_096,
            permit_workspace_writes: false,
            permit_external_effects: false,
        },
        &AllowReadOnlyApproval,
    )
    .await
    .expect("runaway context is trimmed, not aborted");
    assert_eq!(output.answer, "done");
    // The bound still binds: every turn's assembled conversation fits the byte
    // budget. This is the invariant the old aborting test guarded — the
    // mechanism changed from fail-closed to trim, the bound did not go away.
    let budget = 4_096usize;
    for request in requests.lock().unwrap().iter() {
        let total = request
            .messages
            .iter()
            .map(|message| {
                message.content.len() + message.role.len() + message_size_overhead(message)
            })
            .sum::<usize>();
        assert!(
            total <= budget,
            "each turn must stay within the {budget}-byte budget; saw {total}"
        );
    }
}
