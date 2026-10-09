use super::*;

/// The extraction call's own token report reaches the event stream named as an
/// extraction call. A turn may spend tokens on the answer and on the post-turn
/// extraction, and they bill differently, so a consumer must be able to tell
/// the two apart — the answering rounds' events are emitted by the agent loop,
/// this one by the runtime that owns the extraction call. Emitted **before**
/// the outcome events (proposals, skip reasons), matching how a tool's cost is
/// reported before what it produced.
#[tokio::test]
async fn the_extraction_call_reports_its_usage_on_the_stream() {
    let root = temp_root("extraction_usage");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;

    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
        knowledge_log: Arc::new(Mutex::new(Vec::new())),
    };
    let extraction_with_usage = ChatResponse::new(ChatMessage::text(
        "assistant",
        r#"{"proposals": [{"object_id": "T0", "slot": "table.alias", "value": "orders", "origin": "user_explicit"}]}"#,
    ));
    let mut extraction_response = extraction_with_usage;
    extraction_response.usage = Some(saya_agent::TokenUsage::new(40, 10));

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
            turn_steps: {
                let mut call = ChatResponse::new(ChatMessage {
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
                });
                call.usage = Some(saya_agent::TokenUsage::new(10, 4));
                let mut answer = ChatResponse::new(ChatMessage::text(
                    "assistant",
                    "The orders table contains customer orders.",
                ));
                answer.usage = Some(saya_agent::TokenUsage::new(25, 30));
                vec![call, answer]
            },
            extraction_response: Ok(extraction_response),
            extraction_calls: Mutex::new(0),
        }),
        registry: registry_for("analytics", &identity),
        failures: Vec::new(),
    };
    let runtime = test_runtime(assisted_memory());
    run_prompt_with_inputs(
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

    let captured = events.lock().unwrap();
    let extraction_usage: Vec<saya_agent::TokenUsage> = captured
        .iter()
        .filter_map(|event| match event {
            AgentEvent::Usage {
                call: saya_agent::UsageCall::Extraction,
                usage,
            } => Some(*usage),
            _ => None,
        })
        .collect();
    assert_eq!(
        extraction_usage.len(),
        1,
        "exactly one extraction usage event: {captured:?}"
    );
    assert_eq!(extraction_usage[0].input_tokens, 40);
    assert_eq!(extraction_usage[0].output_tokens, 10);

    // The answering rounds' reports are on the same stream, named differently,
    // so the two call kinds cannot be confused.
    let answering = captured
        .iter()
        .filter(|event| {
            matches!(
                event,
                AgentEvent::Usage {
                    call: saya_agent::UsageCall::Answer,
                    ..
                }
            )
        })
        .count();
    assert!(
        answering >= 1,
        "the answering calls report too, under their own name: {captured:?}"
    );

    // The extraction report precedes the outcome events it belongs to.
    let usage_index = captured
        .iter()
        .position(|event| matches!(event, AgentEvent::Usage { .. }))
        .expect("a usage event exists");
    let proposed_index = captured
        .iter()
        .position(|event| matches!(event, AgentEvent::KnowledgeProposed { .. }))
        .expect("a proposal was emitted");
    assert!(
        usage_index < proposed_index,
        "usage is reported before the proposals it cost: {captured:?}"
    );
    let _ = fs::remove_dir_all(root);
}

/// An extraction call that produced no response (here: a provider error) emits
/// no usage event — no response means no report, and inventing a zero would
/// read as a free call the provider never accounted.
#[tokio::test]
async fn an_extraction_with_no_response_emits_no_usage_event() {
    let root = temp_root("extraction_no_usage");
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
            turn_steps: vec![ChatResponse::new(ChatMessage::text(
                "assistant",
                "The orders table contains customer orders.",
            ))],
            extraction_response: Err(ProviderError::Request("extraction blew up".into())),
            extraction_calls: Mutex::new(0),
        }),
        registry: registry_for("analytics", &identity),
        failures: Vec::new(),
    };
    let runtime = test_runtime(assisted_memory());
    run_prompt_with_inputs(
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
    .expect("turn completes despite the extraction failure");

    let captured = events.lock().unwrap();
    assert!(
        captured.iter().all(|event| !matches!(
            event,
            AgentEvent::Usage {
                call: saya_agent::UsageCall::Extraction,
                ..
            }
        )),
        "no extraction response means no extraction usage event: {captured:?}"
    );
    let _ = fs::remove_dir_all(root);
}
