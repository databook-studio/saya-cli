use super::*;

// No extraction ceiling; the per-session circuit breaker (owner decisions
// 1-3). Post-turn extraction is awaited directly — no `tokio::time::timeout`
// — and a session's extraction disables itself after two consecutive misses
// (a truncated reply or a stalled/timed-out transport), announced once via
// `KnowledgeLearningDisabled`. The extraction call is distinguished from the
// turn call by the `precision schema knowledge extractor` system-prompt
// marker, the same stable marker `TurnAndExtractionProvider` relies on above.
// ===========================================================================

/// Red test 1: a slow extraction is awaited, not cut off. There is no
/// wall-clock ceiling any more (owner decision 1) — the call above takes
/// long enough to prove it is genuinely awaited, and its proposal is stored;
/// no `KnowledgeLearningSkipped` fires.
#[tokio::test]
async fn a_slow_extraction_is_awaited_not_cut_off() {
    let root = temp_root("no_ceiling_slow");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    let runtime = test_runtime(assisted_memory());

    let started = std::time::Instant::now();
    let (captured, answer) = run_breaker_turn(
        &runtime,
        Box::new(SlowExtractionProvider {
            turn_step: Mutex::new(0),
            turn_steps: breaker_turn_steps("The orders table contains customer orders."),
        }),
        &identity,
        store.clone(),
        None,
    )
    .await;

    assert_eq!(answer, "The orders table contains customer orders.");
    assert!(
        started.elapsed() >= std::time::Duration::from_millis(300),
        "the call was genuinely awaited, not raced against a timeout"
    );
    assert!(
        skipped_reasons(&captured).is_empty(),
        "an awaited extraction must not skip: {captured:?}"
    );
    let proposed = captured
        .iter()
        .filter(|e| matches!(e, AgentEvent::KnowledgeProposed { .. }))
        .count();
    assert_eq!(proposed, 1, "the proposal is stored: {captured:?}");
    let _ = fs::remove_dir_all(root);
}

/// Red test 2: two consecutive truncated extractions (in one session) trip
/// the breaker and say so exactly once; a third turn makes no extraction
/// request at all.
#[tokio::test]
async fn two_consecutive_truncated_extractions_disable_learning_and_say_so() {
    let root = temp_root("breaker_truncated");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    let runtime = test_runtime(assisted_memory());
    let session = Arc::new(crate::interactive::session::universe::SessionUniverse::empty());

    let truncated = || ProviderError::output_truncated(String::new(), Vec::new());
    let miss_provider = || {
        Box::new(TurnAndExtractionProvider {
            turn_step: Mutex::new(0),
            turn_steps: breaker_turn_steps("The orders table contains customer orders."),
            extraction_response: Err(truncated()),
            extraction_calls: Mutex::new(0),
        })
    };

    // Turn 1: one miss. Not disabled yet.
    let (first, _) = run_breaker_turn(
        &runtime,
        miss_provider(),
        &identity,
        store.clone(),
        Some(Arc::clone(&session)),
    )
    .await;
    assert_eq!(skipped_reasons(&first), vec![LearningSkipReason::Failed]);
    assert_eq!(disabled_event(&first), None, "one miss must not trip it");

    // Turn 2: the second consecutive miss trips the breaker.
    let (second, _) = run_breaker_turn(
        &runtime,
        miss_provider(),
        &identity,
        store.clone(),
        Some(Arc::clone(&session)),
    )
    .await;
    assert_eq!(skipped_reasons(&second), vec![LearningSkipReason::Failed]);
    let skipped_at = second
        .iter()
        .position(|e| matches!(e, AgentEvent::KnowledgeLearningSkipped { .. }))
        .expect("skipped fired");
    let disabled_at = second
        .iter()
        .position(|e| matches!(e, AgentEvent::KnowledgeLearningDisabled { .. }))
        .expect("disabled fired");
    assert!(
        skipped_at < disabled_at,
        "KnowledgeLearningDisabled must follow this turn's own KnowledgeLearningSkipped: {second:?}"
    );
    assert_eq!(disabled_event(&second), Some(("test-model", 2)));

    // Turn 3: the breaker is tripped — no extraction request at all. The
    // provider is shared so its own `extraction_calls` counter (not the
    // recording sink) proves the request was never sent — the same
    // shared-provider pattern `a_gate_declined_turn_emits_no_learning_event`
    // uses above.
    let third_provider = Arc::new(TurnAndExtractionProvider {
        turn_step: Mutex::new(0),
        turn_steps: breaker_turn_steps("The orders table contains customer orders."),
        extraction_response: Err(truncated()),
        extraction_calls: Mutex::new(0),
    });
    struct SharedBreakerProvider(Arc<TurnAndExtractionProvider>);
    #[async_trait]
    impl ChatProvider for SharedBreakerProvider {
        fn name(&self) -> &str {
            "shared-breaker-provider"
        }
        async fn complete(&self, req: ChatRequest) -> Result<ChatResponse, ProviderError> {
            self.0.complete(req).await
        }
    }
    let (third, _) = run_breaker_turn(
        &runtime,
        Box::new(SharedBreakerProvider(Arc::clone(&third_provider))),
        &identity,
        store.clone(),
        Some(Arc::clone(&session)),
    )
    .await;
    assert_eq!(
        *third_provider.extraction_calls.lock().unwrap(),
        0,
        "no extraction request is made once the breaker has tripped"
    );
    assert!(
        !third
            .iter()
            .any(|e| matches!(e, AgentEvent::KnowledgeLearningStarted)),
        "no KnowledgeLearningStarted once disabled: {third:?}"
    );
    assert_eq!(
        disabled_event(&third),
        None,
        "KnowledgeLearningDisabled never fires twice: {third:?}"
    );
    let _ = fs::remove_dir_all(root);
}
