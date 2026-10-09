use super::*;

/// 2. Mock provider returns error during extraction; agent output is returned successfully and unaffected (Safety Property 1).
#[tokio::test]
async fn test_runtime_extraction_failure_never_fails_turn() {
    let root = temp_root("extract_fail_safe");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;

    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
        knowledge_log: Arc::new(Mutex::new(Vec::new())),
    };
    let inputs = TurnInputs {
        ai: ResolvedAi {
            provider: AiProvider::Ollama,
            model: "test-model".into(),
            base_url: None,
            api_key: None,
            allow_data_sharing: true,
            temperature: 0.0,
            timeout_seconds: 60,
            idle_timeout_seconds: 90,
            max_output_tokens: 4096,
            max_output_tokens_is_default: true,
            context_byte_budget: 256 * 1024,
            context_window_tokens: None,
            show_thinking: false,
            compaction: saya_config::CompactionMode::Auto,
            retry_delays_ms: vec![250, 500, 1000],
        },
        provider: Box::new(TurnAndExtractionProvider {
            turn_step: Mutex::new(0),
            turn_steps: vec![
                ChatResponse::new(ChatMessage {
                    role: "assistant".into(),
                    content: String::new(),
                    tool_calls: vec![ToolCall {
                        id: "call-1".into(),
                        name: "bounded_sql_query".into(),
                        arguments: serde_json::json!({
                            "connection": "analytics",
                            "sql": "SELECT id, status FROM catalog.public.orders",
                        }),
                    }],
                    tool_call_id: None,
                }),
                ChatResponse::new(ChatMessage::text(
                    "assistant",
                    "The orders table was inspected successfully.",
                )),
            ],
            extraction_response: Err(ProviderError::configuration("http 500 error")),
            extraction_calls: Mutex::new(0),
        }),
        registry: registry_for("analytics", &identity),
        failures: Vec::new(),
    };
    let runtime = test_runtime(assisted_memory());
    let out = run_prompt_with_inputs(
        &runtime,
        inputs,
        "table orders has alias orders",
        saya_agent::ApprovalPolicy::ReadOnly,
        false,
        false,
        Vec::new(),
        &sink,
        saya_agent::CancellationToken::new(),
        Some(store.clone()),
        None,
        None,
        None,
        saya_agent::AgentMode::Build,
        // No capture hook in these headless turn tests (C1).
        None,
        None,
    )
    .await
    .expect("turn completes despite extraction failure (fail-soft)");

    assert_eq!(out.answer, "The orders table was inspected successfully.");

    let captured = events.lock().unwrap();
    let proposed_count = captured
        .iter()
        .filter(|e| matches!(e, AgentEvent::KnowledgeProposed { .. }))
        .count();
    assert_eq!(
        proposed_count, 0,
        "no proposals emitted when extraction fails"
    );

    let _ = fs::remove_dir_all(root);
}

/// 3. When memory.mode = MemoryMode::Off, zero extraction requests occur.
#[tokio::test]
async fn test_runtime_extraction_skipped_when_memory_mode_off() {
    let root = temp_root("extract_off");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;

    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
        knowledge_log: Arc::new(Mutex::new(Vec::new())),
    };
    let provider = Arc::new(TurnAndExtractionProvider {
        turn_step: Mutex::new(0),
        turn_steps: vec![
            ChatResponse::new(ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: "call-1".into(),
                    name: "bounded_sql_query".into(),
                    arguments: serde_json::json!({
                        "connection": "analytics",
                        "sql": "SELECT id, status FROM catalog.public.orders",
                    }),
                }],
                tool_call_id: None,
            }),
            ChatResponse::new(ChatMessage::text("assistant", "query completed")),
        ],
        extraction_response: Ok(ChatResponse::new(ChatMessage::text(
            "assistant",
            r#"{"proposals": []}"#,
        ))),
        extraction_calls: Mutex::new(0),
    });

    struct SharedProvider(Arc<TurnAndExtractionProvider>);
    #[async_trait]
    impl ChatProvider for SharedProvider {
        fn name(&self) -> &str {
            "shared"
        }
        async fn complete(&self, req: ChatRequest) -> Result<ChatResponse, ProviderError> {
            self.0.complete(req).await
        }
    }

    let inputs = TurnInputs {
        ai: ResolvedAi {
            provider: AiProvider::Ollama,
            model: "test-model".into(),
            base_url: None,
            api_key: None,
            allow_data_sharing: true,
            temperature: 0.0,
            timeout_seconds: 60,
            idle_timeout_seconds: 90,
            max_output_tokens: 4096,
            max_output_tokens_is_default: true,
            context_byte_budget: 256 * 1024,
            context_window_tokens: None,
            show_thinking: false,
            compaction: saya_config::CompactionMode::Auto,
            retry_delays_ms: vec![250, 500, 1000],
        },
        provider: Box::new(SharedProvider(provider.clone())),
        registry: registry_for("analytics", &identity),
        failures: Vec::new(),
    };
    let mut mem = assisted_memory();
    mem.mode = saya_config::MemoryMode::Off;
    let runtime = test_runtime(mem);
    let out = run_prompt_with_inputs(
        &runtime,
        inputs,
        "table orders has alias orders",
        saya_agent::ApprovalPolicy::ReadOnly,
        false,
        false,
        Vec::new(),
        &sink,
        saya_agent::CancellationToken::new(),
        Some(store.clone()),
        None,
        None,
        None,
        saya_agent::AgentMode::Build,
        // No capture hook in these headless turn tests (C1).
        None,
        None,
    )
    .await
    .expect("turn completes");

    assert_eq!(out.answer, "query completed");
    assert_eq!(
        *provider.extraction_calls.lock().unwrap(),
        0,
        "extraction was never called"
    );

    let _ = fs::remove_dir_all(root);
}
