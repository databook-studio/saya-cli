use super::*;

#[tokio::test]
async fn a_turn_supplying_claims_emits_one_event_naming_those_claims() {
    let root = temp_root("t1_supply");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    // Seed a confirmed default_time_column claim under a matching live schema.
    let obj = object(&identity, "orders");
    let tree = orders_schema(&identity);
    store
        .upsert_schema(identity.as_str(), &tree.1)
        .await
        .unwrap();
    let fp = live_fingerprint(&orders_table());
    let claim_id = remember_default_time_column(
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
    let runtime = test_runtime(assisted_memory());
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

    let captured = events.lock().unwrap();
    // Exactly one KnowledgeSupplied.
    let count = captured
        .iter()
        .filter(|e| matches!(e, AgentEvent::KnowledgeSupplied { .. }))
        .count();
    assert_eq!(count, 1, "exactly one KnowledgeSupplied: {captured:?}");
    // It names the supplied claim.
    let event = captured
        .iter()
        .find_map(|e| match e {
            AgentEvent::KnowledgeSupplied { contracts, .. } => Some(contracts),
            _ => None,
        })
        .expect("KnowledgeSupplied present");
    assert_eq!(event.len(), 1, "one object supplied: {event:?}");
    assert_eq!(event[0].object, "catalog.public.orders");
    assert_eq!(event[0].profile, "analytics", "name, not identity");
    assert_eq!(event[0].claims.len(), 1);
    assert_eq!(event[0].claims[0].claim_id, claim_id);
    assert_eq!(event[0].claims[0].value, "created_at");
    assert_eq!(event[0].claims[0].status, ClaimStatus::Confirmed);
    let _ = fs::remove_dir_all(root);
}
