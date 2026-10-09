use super::*;

// Test 6: no opaque profile identity leaks into the serialized event. The DTO
// has no identity field by construction; this asserts the event stream inherits
// that guarantee (mirrors the P2d identity-leak test).

// ===========================================================================

// Test 4: a candidate claim contradicted emits nothing (only confirmed claims
// can be overridden). `recall = IncludeCandidates` so the candidate IS supplied
// to the model — the detector's status guard is what suppresses the finding,
// not recall's filter.

#[tokio::test]
async fn a_candidate_claim_contradicted_emits_nothing() {
    let (root, _claim_id, store) =
        a1_turn_setup(ClaimStatus::Candidate, ClaimOrigin::UserExplicit).await;

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
        provider: Box::new(QueryProvider {
            sql: OVERRIDE_SQL,
            calls: Mutex::new(0),
        }),
        registry: registry_for("analytics", &identity_for("analytics")),
        failures: Vec::new(),
    };
    let runtime = test_runtime(a1_include_candidates_memory());
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
    .expect("turn completes");

    let captured = events.lock().unwrap();
    // The candidate IS supplied (IncludeCandidates), so KnowledgeSupplied is
    // present — but no KnowledgeOverridden fires: only a confirmed claim binds.
    assert!(
        captured
            .iter()
            .any(|e| matches!(e, AgentEvent::KnowledgeSupplied { .. })),
        "the candidate was supplied: {captured:?}"
    );
    assert!(
        one_overridden(&captured).is_none(),
        "a candidate claim is not overridable: {captured:?}"
    );
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn no_identity_leaks_into_the_knowledge_overridden_event() {
    let (root, _claim_id, store) =
        a1_turn_setup(ClaimStatus::Confirmed, ClaimOrigin::UserExplicit).await;
    let identity_str = identity_for("analytics").as_str().to_string();

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
        provider: Box::new(QueryProvider {
            sql: OVERRIDE_SQL,
            calls: Mutex::new(0),
        }),
        registry: registry_for("analytics", &identity_for("analytics")),
        failures: Vec::new(),
    };
    let runtime = test_runtime(a1_memory());
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
    .expect("turn completes");

    let captured = events.lock().unwrap();
    let stream_json = serde_json::to_string(captured.as_slice()).unwrap_or_default();
    assert!(
        !stream_json.contains(&identity_str),
        "opaque identity leaked into the event stream: {stream_json}"
    );
    let _ = fs::remove_dir_all(root);
}
