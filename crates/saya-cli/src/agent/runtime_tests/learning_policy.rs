use super::*;

/// Red test 6: with no session (the one-shot `saya ask` path, each candidate
/// attempt), the breaker can never trip — every call gets a fresh one.
#[tokio::test]
async fn without_a_session_the_breaker_never_trips() {
    let root = temp_root("breaker_no_session");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    let runtime = test_runtime(assisted_memory());

    let miss_provider = || {
        Box::new(TurnAndExtractionProvider {
            turn_step: Mutex::new(0),
            turn_steps: breaker_turn_steps("The orders table contains customer orders."),
            extraction_response: Err(ProviderError::output_truncated(String::new(), Vec::new())),
            extraction_calls: Mutex::new(0),
        })
    };

    for label in ["first", "second"] {
        let (captured, _) =
            run_breaker_turn(&runtime, miss_provider(), &identity, store.clone(), None).await;
        assert!(
            captured
                .iter()
                .any(|e| matches!(e, AgentEvent::KnowledgeLearningStarted)),
            "the {label} call still attempts extraction: {captured:?}"
        );
        assert_eq!(
            disabled_event(&captured),
            None,
            "a fresh breaker per call can never trip within one turn: {captured:?}"
        );
    }
    let _ = fs::remove_dir_all(root);
}

/// A gate-declined turn emits **nothing** for learning (decision 2: a gate skip
/// stays silent). Uses an `Off` memory mode so `permit_candidate_writes` is
/// false and the extraction block is never entered — the same path a gate
/// decline would take when the runtime skips it. Asserts no
/// `KnowledgeLearningSkipped` and no `KnowledgeProposed` appears.
#[tokio::test]
async fn a_gate_declined_turn_emits_no_learning_event() {
    let root = temp_root("p54_gate_decline");
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
        // A trivial turn with no tool call and a short answer: the gate would
        // decline (no object activity, <15-char answer). Memory is Off, so the
        // extraction block is never entered regardless — proving the silent path.
        turn_steps: vec![ChatResponse::new(ChatMessage::text("assistant", "ok"))],
        // Never reached: the gate declines before this could be consulted.
        extraction_response: Err(ProviderError::configuration("unused")),
        extraction_calls: Mutex::new(0),
    });
    struct SharedProvider(Arc<TurnAndExtractionProvider>);
    #[async_trait]
    impl ChatProvider for SharedProvider {
        fn name(&self) -> &str {
            "shared-sleeping"
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
        "hi",
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

    assert_eq!(out.answer, "ok");
    let captured = events.lock().unwrap();
    assert!(
        !captured
            .iter()
            .any(|e| matches!(e, AgentEvent::KnowledgeLearningSkipped { .. })),
        "a gate decline must stay silent: {captured:?}"
    );
    assert!(
        !captured
            .iter()
            .any(|e| matches!(e, AgentEvent::KnowledgeProposed { .. })),
        "no proposals on a gate-declined turn: {captured:?}"
    );
    // Extraction was never called — the gate/permit guard held.
    assert_eq!(
        *provider.extraction_calls.lock().unwrap(),
        0,
        "extraction never ran"
    );
    let _ = fs::remove_dir_all(root);
}
