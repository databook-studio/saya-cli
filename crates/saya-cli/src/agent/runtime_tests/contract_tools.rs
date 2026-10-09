use super::*;

/// 4. Asserts contract_propose is absent from DatabaseTools::definitions(...).
#[test]
fn test_contract_propose_tool_not_advertised_to_model() {
    let tools = super::tools::DatabaseTools::definitions(true, true, true, false, true);
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(
        !names.contains(&"contract_propose"),
        "contract_propose must not be advertised"
    );
    assert!(names.contains(&"schema_discovery"));
    assert!(names.contains(&"bounded_sql_query"));
    assert!(names.contains(&"contract_search"));
    assert!(names.contains(&"contract_read"));
}

/// 5. Supplied contract in recall receipt prevents duplicate candidate proposal from being emitted or stored during the turn (Safety Property 4).
#[tokio::test]
async fn test_anti_self_reinforcement_end_to_end() {
    let root = temp_root("anti_self_reinforce_e2e");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;

    // Seed confirmed claim on orders
    let obj = object(&identity, "orders");
    let tree = orders_schema(&identity);
    store
        .upsert_schema(identity.as_str(), &tree.1)
        .await
        .unwrap();
    let fp = live_fingerprint(&orders_table());
    let _ = remember_default_time_column(
        &store,
        &obj,
        &fp,
        "created_at",
        ClaimStatus::Confirmed,
        ClaimOrigin::UserExplicit,
    )
    .await;

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
                            "sql": "SELECT id, created_at FROM catalog.public.orders",
                        }),
                    }],
                    tool_call_id: None,
                }),
                ChatResponse::new(ChatMessage::text("assistant", "Order dates checked.")),
            ],
            // The model re-infers the *same* default-time claim recall already
            // supplied (`default_time_column=created_at` → slot `table.default_time`,
            // value `created_at`), as `assistant_inferred`. Anti-self-reinforcement
            // drops it as an exact duplicate of the supplied claim — the property
            // this test exists for. A different-slot inference would NOT be dropped,
            // so the fixture must duplicate the supplied slot+value to exercise it.
            extraction_response: Ok(ChatResponse::new(ChatMessage::text(
                "assistant",
                r#"{"proposals": [{"object_id": "T0", "slot": "table.default_time", "value": "created_at", "origin": "assistant_inferred"}]}"#,
            ))),
            extraction_calls: Mutex::new(0),
        }),
        registry: registry_for("analytics", &identity),
        failures: Vec::new(),
    };
    let runtime = test_runtime(assisted_memory());
    let out = run_prompt_with_inputs(
        &runtime,
        inputs,
        "show me orders",
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

    assert_eq!(out.answer, "Order dates checked.");

    let captured = events.lock().unwrap();
    let proposed_count = captured
        .iter()
        .filter(|e| matches!(e, AgentEvent::KnowledgeProposed { .. }))
        .count();
    assert_eq!(
        proposed_count, 0,
        "anti-self-reinforcement dropped duplicate inference"
    );

    let _ = fs::remove_dir_all(root);
}
