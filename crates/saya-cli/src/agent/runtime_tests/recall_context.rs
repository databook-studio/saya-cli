use super::*;

// ===========================================================================

#[tokio::test]
async fn recalled_claim_is_user_context_and_privacy_gate_removes_it_from_actual_request() {
    let root = temp_root("ta04_sentinel");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    // The planted claim: a column name unique to this process, so the
    // assertions cannot pass on text the harness or the prompts coincidentally
    // carry.
    let sentinel = format!("zz_sentinel_col_{}", std::process::id());
    // The claim's binding (Column { sentinel, Time }) is satisfied only by a
    // cached schema that names the column, so the tree carries it as temporal
    // and the item classifies `current`.
    let obj = object(&identity, "orders");
    let tree = SchemaTree {
        databases: vec![Database {
            name: "catalog".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables: vec![Table {
                    name: "orders".into(),
                    primary_key: vec![],
                    foreign_keys: vec![],
                    columns: vec![
                        Column {
                            name: "id".into(),
                            data_type: "bigint".into(),
                            nullable: false,
                        },
                        Column {
                            name: "created_at".into(),
                            data_type: "timestamp".into(),
                            nullable: false,
                        },
                        Column {
                            name: sentinel.clone(),
                            data_type: "timestamp".into(),
                            nullable: false,
                        },
                    ],
                }],
            }],
        }],
    };
    store.upsert_schema(identity.as_str(), &tree).await.unwrap();
    let fp = live_fingerprint(&orders_table());
    remember_default_time_column(
        &store,
        &obj,
        &fp,
        &sentinel,
        ClaimStatus::Confirmed,
        ClaimOrigin::UserExplicit,
    )
    .await;

    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
        knowledge_log: Arc::new(Mutex::new(Vec::new())),
    };
    let seen_requests = Arc::new(Mutex::new(Vec::new()));
    let provider = AnswerProvider {
        answer: "The orders table was inspected for you.",
        log: Arc::new(Mutex::new(Vec::new())),
        seen_requests: Arc::clone(&seen_requests),
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

    let seen = seen_requests.lock().unwrap().clone();
    // An empty or missing recording must fail here: every assertion below is
    // vacuous if the provider never received a request.
    assert!(
        !seen.is_empty(),
        "the provider recorded the requests it received"
    );
    for request in &seen {
        assert!(
            request.messages.iter().any(|m| m.role == "system"),
            "every recorded request carries a system message: {request:?}"
        );
    }
    // Both system-message lanes ran: the turn's request and the post-turn
    // extraction request (the gate's object-activity trigger registers the
    // supplied contract, and the answer is non-trivial).
    assert!(
        seen.len() >= 2,
        "the turn request and the extraction request were both sent: {} requests",
        seen.len()
    );
    // The claim IS in the intended untrusted lane: the turn request's user
    // message, inside the labelled recall block.
    let turn = &seen[0];
    assert!(
        turn.messages.iter().any(|m| {
            m.role == "user"
                && m.content.contains(&sentinel)
                && m.content
                    .contains(super::super::super::recall_context::BLOCK_LABEL)
        }),
        "the planted claim reached the user-side recall block: {turn:?}"
    );
    assert!(
        turn.tools
            .iter()
            .any(|tool| tool.name == "bounded_sql_query"),
        "the open sharing baseline advertises query-data tools"
    );
    // The non-negotiable: no system-role message of ANY request carries the
    // claim — neither the turn's assembled prompt nor the extractor's.
    for request in &seen {
        for message in &request.messages {
            assert!(
                !(message.role == "system" && message.content.contains(&sentinel)),
                "a system message carried the recalled claim: {}",
                message.content
            );
        }
    }

    let closed_seen = Arc::new(Mutex::new(Vec::new()));
    let closed_provider = AnswerProvider {
        answer: "done",
        log: Arc::new(Mutex::new(Vec::new())),
        seen_requests: Arc::clone(&closed_seen),
    };
    let closed_inputs = TurnInputs {
        ai: ResolvedAi {
            provider: AiProvider::Anthropic,
            model: "test-model".into(),
            base_url: None,
            api_key: None,
            allow_data_sharing: false,
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
        provider: Box::new(closed_provider),
        registry: registry_for("analytics", &identity),
        failures: Vec::new(),
    };
    run_prompt_with_inputs(
        &runtime,
        closed_inputs,
        "orders by month",
        saya_agent::ApprovalPolicy::ReadOnly,
        false,
        false,
        Vec::new(),
        &sink,
        saya_agent::CancellationToken::new(),
        Some(SqliteStateStore::new(&db)),
        None,
        None,
        None,
        saya_agent::AgentMode::Build,
        None,
        None,
    )
    .await
    .unwrap();

    let closed = closed_seen.lock().unwrap();
    let closed_turn = closed.first().expect("privacy-gated request was sent");
    assert!(
        !closed_turn
            .messages
            .iter()
            .any(|message| message.content.contains(&sentinel)),
        "privacy-gated request must not contain recalled data: {closed_turn:?}"
    );
    assert!(
        !closed_turn
            .tools
            .iter()
            .any(|tool| tool.name == "bounded_sql_query"),
        "privacy-gated request must omit query-data tools"
    );
    assert!(
        closed_turn
            .tools
            .iter()
            .any(|tool| tool.name == "schema_discovery"),
        "privacy-gated request retains no-data schema discovery"
    );
    let _ = fs::remove_dir_all(root);
}
