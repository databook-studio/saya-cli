use super::*;

#[tokio::test]
async fn knowledge_supplied_precedes_the_provider_request() {
    let root = temp_root("t2_order");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    let obj = object(&identity, "orders");
    let tree = orders_schema(&identity);
    store
        .upsert_schema(identity.as_str(), &tree.1)
        .await
        .unwrap();
    let fp = live_fingerprint(&orders_table());
    remember_default_time_column(
        &store,
        &obj,
        &fp,
        "created_at",
        ClaimStatus::Confirmed,
        ClaimOrigin::UserExplicit,
    )
    .await;

    let events = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
        knowledge_log: log.clone(),
    };
    let provider = AnswerProvider {
        answer: "done",
        log: log.clone(),
        seen_requests: Arc::new(Mutex::new(Vec::new())),
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
        provider: Box::new(provider),
        registry: registry_for("analytics", &identity),
        failures: Vec::new(),
    };
    let runtime = test_runtime(default_memory());
    run_prompt_with_inputs(
        &runtime,
        inputs,
        "orders by month",
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
    .unwrap();

    // The shared log records the order of the two events: the sink pushed
    // "knowledge" on KnowledgeSupplied, then the provider pushed "provider"
    // when `complete` was called. "knowledge" first proves the emit preceded
    // the provider request — the point of the slice.
    let log = log.lock().unwrap();
    assert_eq!(
        log.first(),
        Some(&"knowledge"),
        "KnowledgeSupplied was emitted before the provider was called: {log:?}"
    );
    assert!(
        log.contains(&"provider"),
        "the provider was called after the emit: {log:?}"
    );
    let knowledge_idx = log
        .iter()
        .position(|e| *e == "knowledge")
        .expect("knowledge present");
    let provider_idx = log
        .iter()
        .position(|e| *e == "provider")
        .expect("provider present");
    assert!(
        knowledge_idx < provider_idx,
        "emit index {knowledge_idx} must precede provider index {provider_idx}: {log:?}"
    );

    // And in the sink's own event sequence, KnowledgeSupplied precedes the
    // first loop event (AssistantText from the provider's answer).
    let captured = events.lock().unwrap();
    let knowledge_event_idx = captured
        .iter()
        .position(|e| matches!(e, AgentEvent::KnowledgeSupplied { .. }))
        .expect("KnowledgeSupplied in sink");
    let first_loop_idx = captured
        .iter()
        .position(|e| {
            matches!(
                e,
                AgentEvent::AssistantText { .. }
                    | AgentEvent::ToolRequested { .. }
                    | AgentEvent::Complete
            )
        })
        .expect("a loop event after the provider call");
    assert!(
        knowledge_event_idx < first_loop_idx,
        "KnowledgeSupplied (idx {knowledge_event_idx}) must precede the first loop event (idx {first_loop_idx}): {captured:?}"
    );
    let _ = fs::remove_dir_all(root);
}
