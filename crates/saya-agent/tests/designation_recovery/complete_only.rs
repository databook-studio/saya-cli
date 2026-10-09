/// A complete-only provider must preserve prose when the same response also
/// carries the designation tool call.
#[tokio::test]
async fn complete_only_designation_with_prose_uses_one_request_without_execution() {
    struct CompleteOnly {
        requests: Arc<Mutex<Vec<ChatRequest>>>,
    }

    #[async_trait]
    impl ChatProvider for CompleteOnly {
        fn name(&self) -> &str {
            "complete-only"
        }

        async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
            self.requests.lock().unwrap().push(request);
            Ok(ChatResponse::new(ChatMessage {
                role: "assistant".into(),
                content: "The answer is 42.".into(),
                tool_calls: vec![designation_call("d1", "SELECT 42")],
                tool_call_id: None,
            }))
        }
    }

    let requests = Arc::new(Mutex::new(Vec::new()));
    let executed = Arc::new(Mutex::new(Vec::new()));
    let output = run_agent_with_sink(
        &CompleteOnly {
            requests: requests.clone(),
        },
        &ExecutedTools {
            names: executed.clone(),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &NoopEventSink,
        CancellationToken::new(),
    )
    .await
    .expect("a complete-only response with designation and prose completes");

    assert_eq!(output.answer, "The answer is 42.");
    assert_eq!(output.answer_sql.as_deref(), Some("SELECT 42"));
    assert_eq!(requests.lock().unwrap().len(), 1);
    assert!(executed.lock().unwrap().is_empty());
}
