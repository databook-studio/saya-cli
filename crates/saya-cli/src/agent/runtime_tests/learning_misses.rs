use super::*;

/// Red test 3: a stalled stream counts as a miss exactly like truncation —
/// two in a row trip the breaker.
#[tokio::test]
async fn a_stalled_stream_counts_as_a_miss() {
    let root = temp_root("breaker_stalled");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    let runtime = test_runtime(assisted_memory());
    let session = Arc::new(crate::interactive::session::universe::SessionUniverse::empty());

    let stalled_provider = || {
        Box::new(TurnAndExtractionProvider {
            turn_step: Mutex::new(0),
            turn_steps: breaker_turn_steps("The orders table contains customer orders."),
            extraction_response: Err(ProviderError::Request("provider stream stalled".into())),
            extraction_calls: Mutex::new(0),
        })
    };

    let (first, _) = run_breaker_turn(
        &runtime,
        stalled_provider(),
        &identity,
        store.clone(),
        Some(Arc::clone(&session)),
    )
    .await;
    assert_eq!(disabled_event(&first), None);

    let (second, _) = run_breaker_turn(
        &runtime,
        stalled_provider(),
        &identity,
        store.clone(),
        Some(Arc::clone(&session)),
    )
    .await;
    assert_eq!(
        disabled_event(&second),
        Some(("test-model", 2)),
        "two consecutive stalls trip the breaker: {second:?}"
    );
    let _ = fs::remove_dir_all(root);
}

/// Red test 4: a success between two misses resets the count — the session
/// is not disabled, and a fourth turn still extracts.
#[tokio::test]
async fn a_success_between_misses_resets_the_count() {
    let root = temp_root("breaker_success_resets");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    let runtime = test_runtime(assisted_memory());
    let session = Arc::new(crate::interactive::session::universe::SessionUniverse::empty());

    let miss_provider = || {
        Box::new(TurnAndExtractionProvider {
            turn_step: Mutex::new(0),
            turn_steps: breaker_turn_steps("The orders table contains customer orders."),
            extraction_response: Err(ProviderError::output_truncated(String::new(), Vec::new())),
            extraction_calls: Mutex::new(0),
        })
    };
    let success_provider = || {
        Box::new(TurnAndExtractionProvider {
            turn_step: Mutex::new(0),
            turn_steps: breaker_turn_steps("The orders table contains customer orders."),
            extraction_response: Ok(ChatResponse::new(ChatMessage::text(
                "assistant",
                r#"{"proposals": []}"#,
            ))),
            extraction_calls: Mutex::new(0),
        })
    };

    // Miss, success, miss.
    let (_, _) = run_breaker_turn(
        &runtime,
        miss_provider(),
        &identity,
        store.clone(),
        Some(Arc::clone(&session)),
    )
    .await;
    let (_, _) = run_breaker_turn(
        &runtime,
        success_provider(),
        &identity,
        store.clone(),
        Some(Arc::clone(&session)),
    )
    .await;
    let (third, _) = run_breaker_turn(
        &runtime,
        miss_provider(),
        &identity,
        store.clone(),
        Some(Arc::clone(&session)),
    )
    .await;
    assert_eq!(
        disabled_event(&third),
        None,
        "a success between misses resets the count: {third:?}"
    );

    // A fourth turn still extracts: a real proposal is stored.
    let (fourth, _) = run_breaker_turn(
        &runtime,
        Box::new(TurnAndExtractionProvider {
            turn_step: Mutex::new(0),
            turn_steps: breaker_turn_steps("The orders table contains customer orders."),
            extraction_response: Ok(ChatResponse::new(ChatMessage::text(
                "assistant",
                r#"{"proposals": [{"object_id": "T0", "slot": "table.alias", "value": "orders", "origin": "user_explicit"}]}"#,
            ))),
            extraction_calls: Mutex::new(0),
        }),
        &identity,
        store.clone(),
        Some(Arc::clone(&session)),
    )
    .await;
    assert!(
        fourth
            .iter()
            .any(|e| matches!(e, AgentEvent::KnowledgeLearningStarted)),
        "the fourth turn still attempts extraction: {fourth:?}"
    );
    let proposed = fourth
        .iter()
        .filter(|e| matches!(e, AgentEvent::KnowledgeProposed { .. }))
        .count();
    assert_eq!(
        proposed, 1,
        "the fourth turn's proposal is stored: {fourth:?}"
    );
    let _ = fs::remove_dir_all(root);
}

/// Red test 5: a non-miss failure (a non-JSON reply) between two misses also
/// resets the count.
#[tokio::test]
async fn a_non_miss_failure_resets_the_count() {
    let root = temp_root("breaker_non_miss_resets");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    let runtime = test_runtime(assisted_memory());
    let session = Arc::new(crate::interactive::session::universe::SessionUniverse::empty());

    let miss_provider = || {
        Box::new(TurnAndExtractionProvider {
            turn_step: Mutex::new(0),
            turn_steps: breaker_turn_steps("The orders table contains customer orders."),
            extraction_response: Err(ProviderError::output_truncated(String::new(), Vec::new())),
            extraction_calls: Mutex::new(0),
        })
    };
    // A parse failure: the reply's first visible character is not JSON.
    let parse_failure_provider = || {
        Box::new(TurnAndExtractionProvider {
            turn_step: Mutex::new(0),
            turn_steps: breaker_turn_steps("The orders table contains customer orders."),
            extraction_response: Ok(ChatResponse::new(ChatMessage::text(
                "assistant",
                "not json at all",
            ))),
            extraction_calls: Mutex::new(0),
        })
    };

    let (_, _) = run_breaker_turn(
        &runtime,
        miss_provider(),
        &identity,
        store.clone(),
        Some(Arc::clone(&session)),
    )
    .await;
    let (parse_failed, _) = run_breaker_turn(
        &runtime,
        parse_failure_provider(),
        &identity,
        store.clone(),
        Some(Arc::clone(&session)),
    )
    .await;
    assert_eq!(
        skipped_reasons(&parse_failed),
        vec![LearningSkipReason::Failed]
    );
    // The parse failure must not count as the second miss: had it tripped
    // the breaker here, turn 3 would make no attempt and emit nothing, and a
    // check on turn 3's events alone would pass vacuously.
    assert_eq!(
        disabled_event(&parse_failed),
        None,
        "a parse failure is not a miss: {parse_failed:?}"
    );
    let (third, _) = run_breaker_turn(
        &runtime,
        miss_provider(),
        &identity,
        store.clone(),
        Some(Arc::clone(&session)),
    )
    .await;
    assert!(
        third
            .iter()
            .any(|e| matches!(e, AgentEvent::KnowledgeLearningStarted)),
        "learning must still be enabled on turn 3, so extraction is attempted: {third:?}"
    );
    assert_eq!(
        disabled_event(&third),
        None,
        "a non-miss failure between misses resets the count: {third:?}"
    );
    let _ = fs::remove_dir_all(root);
}
