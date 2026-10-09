/// A single oversized tool result must not be able to abort the whole run by
/// itself. One query returning more than the conversation
/// byte budget must let the run continue in some useful form rather than
/// surfacing an opaque `Limit("context bytes")` after the query already ran.
/// The model must also be told it received a cut result, not silently fed a
/// partial one as if complete.
#[tokio::test]
async fn single_oversized_tool_result_does_not_abort_the_run() {
    struct BigTools;
    #[async_trait::async_trait]
    impl ToolExecutor for BigTools {
        async fn execute(
            &self,
            _: &str,
            _: serde_json::Value,
        ) -> Result<serde_json::Value, ToolError> {
            // One result larger than the whole conversation budget.
            Ok(serde_json::json!({ "rows": vec!["x".repeat(8_192)] }))
        }
    }
    struct CapturingProvider {
        requests: Arc<Mutex<Vec<ChatRequest>>>,
        turn: Mutex<usize>,
    }
    #[async_trait]
    impl ChatProvider for CapturingProvider {
        fn name(&self) -> &str {
            "capture-big"
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
                    tool_calls: vec![ToolCall {
                        id: "call-1".into(),
                        name: "schema_discovery".into(),
                        arguments: serde_json::json!({}),
                    }],
                    tool_call_id: None,
                }));
            }
            Ok(ChatResponse::new(ChatMessage::text(
                "assistant",
                "summarised the result",
            )))
        }
    }

    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = CapturingProvider {
        requests: requests.clone(),
        turn: Mutex::new(0),
    };
    let output = run_agent(
        &provider,
        &BigTools,
        request(),
        definitions(),
        AgentLimits {
            max_turns: Some(4),
            max_tool_calls: Some(8),
            max_continuations: None,
            permit_candidate_writes: false,
            context_byte_budget: 4_096,
            permit_workspace_writes: false,
            permit_external_effects: false,
        },
        &AllowReadOnlyApproval,
    )
    .await
    // Before the fix this returned `Err(AgentError::Limit("context bytes"))`.
    .expect("one oversized result must not abort the run");
    assert_eq!(output.answer, "summarised the result");
    // The model must not silently believe it saw the complete result.
    assert!(
        output.events.iter().any(
            |event| matches!(event, AgentEvent::ToolCompleted { summary, .. } if summary
                .contains("truncated"))
        ),
        "the model must be told the result was truncated; events: {:?}",
        output.events
    );
    // And the tool message the model actually receives must carry the visible
    // truncation marker, proving it is not misled about what it got.
    let second = requests.lock().unwrap()[1].clone();
    let tool_message = second
        .messages
        .iter()
        .find(|message| message.role == "tool")
        .expect("the truncated result must reach the model as a tool message");
    assert!(
        tool_message.content.contains("truncated"),
        "the tool message content must mark the cut, got: {}",
        tool_message.content
    );
}
