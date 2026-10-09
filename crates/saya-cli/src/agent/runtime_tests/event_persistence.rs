use super::*;

// Test 6: store unavailable still runs the turn and still emits (a Ran event
// with store_unavailable: true). Driven through the runtime with an
// unopenable store so recall fails soft.
// ===========================================================================

// ===========================================================================
// Test 7: no opaque ProfileIdentity value appears in a serialized event.
// Asserted directly on the serialized JSON — the identity is a hash over
// connection material and must not reach output.
// ===========================================================================

// ===========================================================================
// Test 8: the event round-trips through serde with its `type` tag.
// ===========================================================================

// ===========================================================================

#[tokio::test]
async fn store_unavailable_still_runs_the_turn_and_emits() {
    let root = temp_root("t6_store_unavailable");
    fs::write(root.join("blocker"), b"x").unwrap();
    let bad_path = root.join("blocker/state.sqlite3");
    let store = SqliteStateStore::new(&bad_path); // parent is a file → unopenable
    let identity = identity_for("analytics");

    let events = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
        knowledge_log: log.clone(),
    };
    let provider = AnswerProvider {
        answer: "done anyway",
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
    let result = run_prompt_with_inputs(
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
    .await;

    // The turn still completes despite the store failure (recall is fail-soft).
    assert!(
        result.is_ok(),
        "turn completes despite store failure: {result:?}"
    );
    let captured = events.lock().unwrap();
    // The event records the failure as Ran { store_unavailable: true }, not as
    // an error and not as silence.
    let event = captured
        .iter()
        .find_map(|e| match e {
            AgentEvent::KnowledgeSupplied { outcome, .. } => Some(*outcome),
            _ => None,
        })
        .expect("KnowledgeSupplied emitted despite store failure");
    assert_eq!(
        event,
        KnowledgeOutcome::Ran {
            store_unavailable: true
        },
        "store failure is Ran{{store_unavailable: true}}, not silence: {captured:?}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn no_opaque_profile_identity_value_appears_in_the_event() {
    let identity = identity_for("analytics");
    let receipt = RecallReceipt {
        kind: RecallOutcomeKind::Ran {
            store_unavailable: false,
        },
        supplied: vec![crate::contracts::SuppliedContract {
            profile: "analytics".into(),
            object: "catalog.public.orders".into(),
            schema_state: "current",
            claims: vec![crate::contracts::SuppliedClaim {
                claim_id: ClaimId::parse("c-abc123").unwrap(),
                kind: "default_time_column",
                value: "created_at".into(),
                column: None,
                status: ClaimStatus::Confirmed,
            }],
        }],
        dropped_by_bounds: 0,
    };
    let event = knowledge_supplied_event(&receipt);
    let json = serde_json::to_string(&event).expect("serializes");
    // The opaque identity string appears nowhere in the serialized event.
    assert!(
        !json.contains(identity.as_str()),
        "opaque identity leaked into the serialized event: {json}"
    );
    // The human-facing name does appear, in place of the identity.
    assert!(
        json.contains("analytics"),
        "the profile name (not the identity) is what the event carries: {json}"
    );
}

#[test]
fn knowledge_supplied_round_trips_through_serde_with_type_tag() {
    let event = AgentEvent::knowledge_supplied(
        KnowledgeOutcome::Ran {
            store_unavailable: true,
        },
        vec![SuppliedContractDto {
            profile: "analytics".into(),
            object: "catalog.public.orders".into(),
            schema_state: "needs_review".into(),
            claims: vec![SuppliedClaimDto {
                claim_id: ClaimId::parse("c-roundtrip").unwrap(),
                kind: "default_time_column".into(),
                value: "created_at".into(),
                column: Some("created_at".into()),
                status: ClaimStatus::Candidate,
            }],
        }],
        3,
    );
    let json = serde_json::to_string(&event).expect("serializes");
    // The outer tag is `knowledge_supplied` (snake_case, per AgentEvent's
    // `#[serde(tag = "type", rename_all = "snake_case")]`).
    assert!(
        json.contains(r#""type":"knowledge_supplied""#),
        "carries the type tag: {json}"
    );
    let back: AgentEvent = serde_json::from_str(&json).expect("deserializes back");
    assert_eq!(back, event, "round-trips with the claims and status intact");

    // The three outcomes each round-trip too.
    for (outcome, expected) in [
        (KnowledgeOutcome::Off, r#""off""#),
        (KnowledgeOutcome::Skipped, r#""skipped""#),
        (
            KnowledgeOutcome::Ran {
                store_unavailable: false,
            },
            r#"{"ran":{"store_unavailable":false}}"#,
        ),
    ] {
        let text = serde_json::to_string(&outcome).expect("serializes");
        assert_eq!(text, expected, "outcome {outcome:?} serializes as expected");
        let back: KnowledgeOutcome = serde_json::from_str(&text).expect("deserializes back");
        assert_eq!(back, outcome, "outcome {outcome:?} round-trips");
    }
}
