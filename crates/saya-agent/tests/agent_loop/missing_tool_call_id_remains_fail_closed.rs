#[tokio::test]
async fn missing_tool_call_id_remains_fail_closed() {
    let provider = MockProvider {
        responses: Mutex::new(vec![ChatResponse::new(ChatMessage {
            role: "assistant".into(),
            content: String::new(),
            tool_calls: vec![
                ToolCall {
                    id: String::new(),
                    name: "shell".into(),
                    arguments: serde_json::json!({}),
                },
                ToolCall {
                    id: "valid-call".into(),
                    name: "schema_discovery".into(),
                    arguments: serde_json::json!({}),
                },
            ],
            tool_call_id: None,

            images: Vec::new(),
                    reasoning: None,
                    reasoning_signature: None,
        })]),
    };
    #[derive(Default)]
    struct Diagnostics(Mutex<Vec<saya_agent::ToolDiagnostic>>);
    #[async_trait]
    impl AgentEventSink for Diagnostics {
        async fn emit(&self, _: AgentEvent) {}
        async fn observe_tool(&self, diagnostic: saya_agent::ToolDiagnostic) {
            self.0.lock().unwrap().push(diagnostic);
        }
    }
    let sink = Diagnostics::default();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let error = run_agent_with_sink(
        &provider,
        &MockTools {
            calls: calls.clone(),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &sink,
        CancellationToken::new(),
    )
    .await
    .unwrap_err();
    // Without a call id there is no way to anchor a well-formed tool result,
    // so the conversation could not continue validly.
    assert!(matches!(error, AgentError::InvalidToolCall));
    use saya_agent::ToolDiagnosticStatus::{Failed, Refused, Requested};
    let records = sink.0.lock().unwrap();
    assert_eq!(
        records
            .iter()
            .map(|record| record.status())
            .collect::<Vec<_>>(),
        vec![Requested, Failed, Requested, Refused]
    );
    assert_eq!(
        records
            .iter()
            .map(|record| record.tool())
            .collect::<Vec<_>>(),
        vec![
            "unknown_tool",
            "unknown_tool",
            "schema_discovery",
            "schema_discovery"
        ]
    );
    assert!(
        calls.lock().unwrap().is_empty(),
        "invalid message cannot execute its valid sibling"
    );
}
