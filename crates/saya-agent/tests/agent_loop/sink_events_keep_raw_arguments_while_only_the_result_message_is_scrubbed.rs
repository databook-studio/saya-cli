/// Guard against over-redaction (M2-3b): only the model-bound tool *result*
/// message is scrubbed. The user-facing surfaces keep raw bytes — the sink's
/// `ToolRequested` event carries the call's arguments verbatim (the one sink
/// surface that carries raw call data — no sink event carries tool results),
/// and the replayed assistant message keeps its raw arguments so the
/// conversation the provider sees is unchanged on the argument side.
#[tokio::test]
async fn sink_events_keep_raw_arguments_while_only_the_result_message_is_scrubbed() {
    struct PlainTools;
    #[async_trait::async_trait]
    impl ToolExecutor for PlainTools {
        async fn execute(
            &self,
            _: &str,
            _: serde_json::Value,
        ) -> Result<serde_json::Value, ToolError> {
            Ok(serde_json::json!({"ok": true}))
        }
    }
    struct SentinelCallProvider {
        requests: Arc<Mutex<Vec<ChatRequest>>>,
    }
    #[async_trait]
    impl ChatProvider for SentinelCallProvider {
        fn name(&self) -> &str {
            "sentinel-call"
        }
        async fn complete(
            &self,
            request: ChatRequest,
        ) -> Result<ChatResponse, saya_agent::ProviderError> {
            let first = self.requests.lock().unwrap().is_empty();
            self.requests.lock().unwrap().push(request);
            if first {
                Ok(ChatResponse::new(ChatMessage {
                    role: "assistant".into(),
                    content: String::new(),
                    tool_calls: vec![ToolCall {
                        id: "call-1".into(),
                        name: "schema_discovery".into(),
                        arguments: serde_json::json!({"sql": "SELECT 'api_key=sk-live-SENTINEL'"}),
                    }],
                    tool_call_id: None,
                }))
            } else {
                Ok(ChatResponse::new(ChatMessage::text("assistant", "done")))
            }
        }
    }
    let requests = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: Arc::new(Mutex::new(Vec::new())),
    };
    run_agent_with_sink(
        &SentinelCallProvider {
            requests: requests.clone(),
        },
        &PlainTools,
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &sink,
        CancellationToken::new(),
    )
    .await
    .expect("the run completes");
    let requested = sink
        .events
        .lock()
        .unwrap()
        .iter()
        .find_map(|event| match event {
            AgentEvent::ToolRequested { arguments, .. } => Some(arguments.clone()),
            _ => None,
        })
        .expect("the tool call was surfaced to the sink");
    assert!(
        requested.to_string().contains("sk-live-SENTINEL"),
        "the sink event must keep the raw arguments: {requested}"
    );
    let second = requests.lock().unwrap()[1].clone();
    let replay = second
        .messages
        .iter()
        .find(|message| message.role == "assistant" && !message.tool_calls.is_empty())
        .expect("the assistant turn is replayed to the provider");
    let replayed_arguments = serde_json::to_string(&replay.tool_calls).unwrap();
    assert!(
        replayed_arguments.contains("sk-live-SENTINEL"),
        "replayed arguments are never scrubbed: {replayed_arguments}"
    );
}
