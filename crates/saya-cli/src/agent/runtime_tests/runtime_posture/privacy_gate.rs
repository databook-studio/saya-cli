use super::*;

#[tokio::test]
async fn runtime_turn_with_closed_privacy_gate_emits_knowledge_outcome_skipped() {
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
    let identity = identity_for("analytics");
    let inputs = TurnInputs {
        ai: ResolvedAi {
            provider: AiProvider::Anthropic,
            model: "test-model".into(),
            base_url: None,
            api_key: None,
            allow_data_sharing: false, // privacy gate closed for cloud providers
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
        None,
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
    let outcome = captured
        .iter()
        .find_map(|e| match e {
            AgentEvent::KnowledgeSupplied { outcome, .. } => Some(*outcome),
            _ => None,
        })
        .expect("KnowledgeSupplied present");
    assert_eq!(outcome, KnowledgeOutcome::Skipped);
}
