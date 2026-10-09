/// M2-3(b): a tool result entering the model's context is scrubbed at the
/// context boundary. A tool returning `api_key=sk-live-SENTINEL` must reach
/// the provider as `api_key=[redacted]` — the loop byte-caps tool results but
/// never scrubbed them, so the secret shape previously reached the model
/// verbatim. SQL rows are included by design (D8): every tool result entering
/// model context is scrubbed, and the answer may quote `[redacted]`.
#[tokio::test]
async fn tool_result_reaching_the_provider_is_redacted() {
    struct SentinelTools;
    #[async_trait::async_trait]
    impl ToolExecutor for SentinelTools {
        async fn execute(
            &self,
            _: &str,
            _: serde_json::Value,
        ) -> Result<serde_json::Value, ToolError> {
            Ok(serde_json::json!({"rows": [["api_key=sk-live-SENTINEL"]]}))
        }
    }
    let requests = Arc::new(Mutex::new(Vec::new()));
    run_agent(
        &ToolThenAnswerProvider {
            requests: requests.clone(),
        },
        &SentinelTools,
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .expect("the run completes");
    let second = requests.lock().unwrap()[1].clone();
    let tool_message = second
        .messages
        .iter()
        .find(|message| message.role == "tool")
        .expect("the tool result reaches the model as a tool message");
    assert!(
        !tool_message.content.contains("sk-live-SENTINEL"),
        "secret-shaped tool result reached the provider unredacted: {}",
        tool_message.content
    );
    assert!(
        tool_message.content.contains("api_key=[redacted]"),
        "the value must be replaced by the redaction marker, got: {}",
        tool_message.content
    );
}
