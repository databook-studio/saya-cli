use super::*;

// Chunk 4: Post-turn structured extraction runtime integration tests
// ===========================================================================

/// 1. Integration test: agent runs, completes answer, harness executes extraction,
/// writes to store, and sink receives KnowledgeProposed.
#[tokio::test]
async fn test_runtime_runs_post_turn_extraction_and_emits_proposed_event() {
    let root = temp_root("post_turn_extract");
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
                    "The orders table contains customer orders.",
                )),
            ],
            extraction_response: Ok(ChatResponse::new(ChatMessage::text(
                "assistant",
                r#"{"proposals": [{"object_id": "T0", "slot": "table.alias", "value": "orders", "origin": "user_explicit"}]}"#,
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

    assert_eq!(out.answer, "The orders table contains customer orders.");

    // The event assertions run under the sink lock; the store query below
    // awaits, so the guard is dropped before it (holding a std `Mutex` guard
    // across an await is a clippy error and a real footgun).
    {
        let captured = events.lock().unwrap();
        let proposed: Vec<&ProposedClaimDto> = captured
            .iter()
            .filter_map(|event| match event {
                AgentEvent::KnowledgeProposed { claim } => Some(claim),
                _ => None,
            })
            .collect();
        assert_eq!(proposed.len(), 1, "exactly one KnowledgeProposed emitted");
        assert_eq!(proposed[0].profile, "analytics");
        assert_eq!(proposed[0].object, "catalog.public.orders");
        assert_eq!(proposed[0].kind, "table_alias");
        assert_eq!(proposed[0].value, "orders");
        // The user explicitly asserted the alias, so
        // `ClaimOrigin::may_confirm_directly` the proposal lands `Active`, which
        // the DTO reports as `Confirmed` — a user assertion is the act of
        // confirmation, not a candidate pending it.
        assert_eq!(proposed[0].status, ClaimStatus::Confirmed);
    }

    let obj = object(&identity, "orders");
    // Phase F persists to `knowledge_items` (D-3's projection), not the legacy
    // `contract_claims` table the retired `contract_propose` wrote to — so the
    // end-to-end persistence is asserted through the knowledge-items read.
    let stored = store.knowledge_for_object(&obj).await.unwrap();
    assert_eq!(stored.len(), 1, "persisted in knowledge_items");
    assert_eq!(stored[0].slot, KnowledgeSlot::TableAlias);
    assert_eq!(stored[0].state, KnowledgeState::Active);
    assert_eq!(stored[0].source, ClaimOrigin::UserExplicit);

    let _ = fs::remove_dir_all(root);
}
