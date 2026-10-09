use super::*;

/// The agent loop assigns each serial result a share before it calls the
/// production `RunTools` wrapper. Capture uses that exact share, so it refuses
/// a result the provider receives cut even when it would fit alone.
#[tokio::test]
async fn run_tools_forwards_serial_turn_share_to_capture() {
    struct Provider {
        requests: Arc<Mutex<Vec<ChatRequest>>>,
        turn: Mutex<usize>,
    }

    #[async_trait]
    impl ChatProvider for Provider {
        fn name(&self) -> &str {
            "capture-share"
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
                    tool_calls: ["one", "two", "three", "four", "five"]
                        .into_iter()
                        .map(|id| ToolCall {
                            id: id.into(),
                            name: "bounded_sql_query".into(),
                            arguments: serde_json::json!({
                                "sql": "SELECT hex(randomblob(30000)) AS payload"
                            }),
                        })
                        .collect(),
                    tool_call_id: None,
                }));
            }
            Ok(ChatResponse::new(ChatMessage::text("assistant", "done")))
        }
    }

    let (connector, dir) = sqlite_fixture().await;
    let (hook, observed) = observing_hook();
    let database =
        DatabaseTools::new(Some(connector), 100, true).with_capture(Some(hook), 300 * 1024);
    let tools = RunTools::compose(Arc::new(database), None, None, None);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let output = run_agent_with_sink(
        &Provider {
            requests: requests.clone(),
            turn: Mutex::new(0),
        },
        &tools,
        AgentRequest {
            prompt: "show payloads".into(),
            profile_names: vec!["primary".into()],
            model: "capture-share".into(),
            system_prompt: None,
            history: Vec::new(),
            context_blocks: Vec::new(),
        },
        vec![ToolDefinition {
            name: "bounded_sql_query".into(),
            description: "query".into(),
            read_only: true,
            parameters: serde_json::json!({"type": "object"}),
            effect: ToolEffect {
                database_data: true,
                external_side_effect: false,
                requires_approval: true,
                local_state: LocalStateEffect::None,
            },
            concurrency: ToolConcurrency::Serial,
            completion: None,
        }],
        AgentLimits {
            context_byte_budget: 300 * 1024,
            ..AgentLimits::default()
        },
        &AllowReadOnlyApproval,
        &saya_agent::NoopEventSink,
        saya_agent::CancellationToken::new(),
    )
    .await
    .expect("serial tool turn completes");
    assert_eq!(output.answer, "done");
    let provider_view = requests.lock().unwrap()[1].clone();
    let tool_messages: Vec<_> = provider_view
        .messages
        .iter()
        .filter(|message| message.role == "tool")
        .collect();
    assert_eq!(tool_messages.len(), 5);
    assert!(
        tool_messages
            .iter()
            .all(|message| message.content.contains("[truncated]")),
        "provider views: {:?}",
        tool_messages
            .iter()
            .map(|message| &message.content)
            .collect::<Vec<_>>()
    );
    let events = observed.lock().unwrap();
    assert_eq!(events.len(), 5);
    assert!(events.iter().all(|event| {
        matches!(
            event,
            CaptureEvent::Refused {
                reason: CaptureRefusalReason::ModelViewTruncated,
                ..
            }
        )
    }));
    let _ = std::fs::remove_dir_all(dir);
}
