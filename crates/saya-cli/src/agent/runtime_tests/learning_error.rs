use super::*;

/// A turn whose extraction *errors* (provider failure) emits
/// `KnowledgeLearningSkipped { Failed }` — the non-timeout arm — and still
/// completes. Reuses `TurnAndExtractionProvider` with an `Err` extraction
/// response, the same harness `test_runtime_extraction_failure_never_fails_turn`
/// uses, but asserts the new event (the older test predates it and only
/// asserts no proposals).
#[tokio::test]
async fn a_turn_whose_extraction_errors_emits_learning_skipped_failed_and_completes() {
    let root = temp_root("p54_failed");
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
        Some(store),
        None,
        None,
        None,
        saya_agent::AgentMode::Build,
        // No capture hook in these headless turn tests (C1).
        None,
        None,
    )
    .await
    .expect("turn completes despite extraction error (fail-soft)");

    assert_eq!(out.answer, "The orders table was inspected successfully.");

    let captured = events.lock().unwrap();
    let skipped = captured.iter().find_map(|event| match event {
        AgentEvent::KnowledgeLearningSkipped { reason } => Some(*reason),
        _ => None,
    });
    assert_eq!(
        skipped,
        Some(LearningSkipReason::Failed),
        "error must emit KnowledgeLearningSkipped{{Failed}}, not TimedOut: {captured:?}"
    );
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(root);
}
