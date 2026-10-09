/// A failing tool's error text must reach the model as the tool result so it
/// can adjust (e.g. a safety-rejection reason), not be flattened to a generic
/// "database tool failed" blob.
#[tokio::test]
async fn tool_failure_details_reach_the_model() {
    struct CapturingProvider2 {
        requests: Arc<Mutex<Vec<ChatRequest>>>,
    }

    struct FailingTools;

    #[async_trait::async_trait]
    impl ToolExecutor for FailingTools {
        async fn execute(
            &self,
            _: &str,
            _: serde_json::Value,
        ) -> Result<serde_json::Value, ToolError> {
            Err(ToolError::QueryFailedDetail(
                "query rejected: DELETE modifies data".into(),
            ))
        }
    }

    #[async_trait::async_trait]
    impl ChatProvider for CapturingProvider2 {
        fn name(&self) -> &str {
            "capture"
        }
        async fn complete(
            &self,
            request: ChatRequest,
        ) -> Result<ChatResponse, saya_agent::ProviderError> {
            let mut turns = self.requests.lock().unwrap();
            let turn = turns.len();
            turns.push(request);
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
                "adjusted",
            )))
        }
    }

    let provider = CapturingProvider2 {
        requests: Arc::new(Mutex::new(Vec::new())),
    };
    let output = run_agent(
        &provider,
        &FailingTools,
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .unwrap();
    assert_eq!(output.answer, "adjusted");
    let second = provider.requests.lock().unwrap()[1].clone();
    let tool_message = second
        .messages
        .iter()
        .find(|message| message.role == "tool")
        .expect("the failure must be fed back as a tool result");
    assert!(
        tool_message.content.contains("DELETE modifies data"),
        "model must see the underlying reason, got: {}",
        tool_message.content
    );
}
