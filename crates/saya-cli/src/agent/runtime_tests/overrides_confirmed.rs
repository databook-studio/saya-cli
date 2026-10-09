use super::*;

// Test 2: a turn whose SQL honours the claim emits nothing.

// Test 3: unparseable SQL emits nothing.

// Spec A1: surfacing an override from the SQL, not the model's confession.
//
// `detect_overrides` exists and is tested against the real generated SQL; this
// slice wires it into the turn and emits `KnowledgeOverridden`. These tests
// drive a turn through `run_prompt_with_inputs` with a provider that issues one
// `bounded_sql_query` call, so detection runs against the statement the model
// actually generated and the receipt recall supplied — not a unit oracle.
//
// The harness mirrors test 1 above: a confirmed `default_time_column` claim of
// `return_date` seeded under a matching cached `orders` schema (columns
// `id` + `created_at`), so recall supplies the claim as `Current`. The claim
// names `return_date` as the time column; the model's SQL references a
// *different* time-named column (`rental_date`) on the same object — the live
// override case the detector exists to catch.
// ===========================================================================

#[tokio::test]
async fn a_turn_contradicting_a_confirmed_claim_emits_one_knowledge_overridden() {
    let (root, claim_id, store) =
        a1_turn_setup(ClaimStatus::Confirmed, ClaimOrigin::UserExplicit).await;

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
        // The store must be present so recall supplies the claim.
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
    let findings = one_overridden(&captured).expect("one KnowledgeOverridden");
    assert_eq!(
        findings.len(),
        1,
        "exactly one finding, naming the contradicted claim: {captured:?}"
    );
    let f = &findings[0];
    assert_eq!(f.claim_id, claim_id, "names the supplied confirmed claim");
    assert_eq!(f.kind, "default_time_column");
    assert_eq!(f.claimed_value, "return_date", "where you specified Y");
    assert!(
        f.observed_columns.contains(&"rental_date".to_string()),
        "names the column actually referenced: {f:?}"
    );
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn a_turn_honouring_the_claim_emits_no_knowledge_overridden() {
    let (root, _claim_id, store) =
        a1_turn_setup(ClaimStatus::Confirmed, ClaimOrigin::UserExplicit).await;
    // The claimed column `return_date` is referenced → the claim is honoured.
    let honoring_sql = "SELECT return_date FROM orders WHERE return_date > '2024-01-01'";

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
            sql: honoring_sql,
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
    assert!(
        one_overridden(&captured).is_none(),
        "honouring the claim must emit no override event: {captured:?}"
    );
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn a_turn_with_unparseable_sql_emits_no_knowledge_overridden() {
    let (root, _claim_id, store) =
        a1_turn_setup(ClaimStatus::Confirmed, ClaimOrigin::UserExplicit).await;
    // `sql_references` returns `None` for this; the detector fails closed.
    let unparseable_sql = "SELECT FROM WHERE";

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
            sql: unparseable_sql,
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
    assert!(
        one_overridden(&captured).is_none(),
        "unparseable SQL must emit no override event: {captured:?}"
    );
    let _ = fs::remove_dir_all(root);
}
