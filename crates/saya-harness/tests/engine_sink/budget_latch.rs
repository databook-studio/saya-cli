use super::*;

#[tokio::test]
async fn known_usage_budget_is_latched_before_waiting_for_display() {
    let clock = SimClock::new();
    let root = temp_root("budget-before-display");
    let (store, run_id) = seeded_store(&root, "budget-before-display", RunStatus::Executing).await;
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).unwrap();
    let clock_for_sink = clock.clone();
    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let cancellation = saya_agent::CancellationToken::default();
    let sink = EngineEventSink::new(
        run_id,
        RunState::Executing,
        Journal::open(&run_dir),
        store,
        SinkBudgets {
            wall_clock: Some(Duration::from_millis(100)),
            token_ceiling: Some(1),
            download_budget: None,
            carried_usage: UsageTotals::default(),
        },
        move || clock_for_sink.clock(),
    )
    .with_cancellation(cancellation.clone())
    .with_agent_stream(Arc::new(HeldDisplay {
        entered: std::sync::Mutex::new(Some(entered_tx)),
        release: std::sync::Mutex::new(Some(release_rx)),
    }));
    let event = AgentEvent::Usage {
        call: UsageCall::Answer,
        usage: TokenUsage::new(1, 0),
    };
    let emit = AgentEventSink::emit(&sink, event);
    tokio::pin!(emit);

    tokio::select! {
        entered = entered_rx => entered.unwrap(),
        _ = &mut emit => panic!("display did not retain its source future"),
    }
    assert_eq!(
        cancellation.reason(),
        Some(saya_agent::CancellationReason::Budget(
            saya_agent::BudgetDimension::ReportedTokens
        )),
        "the known receipt owns the stop before downstream display can wait"
    );
    assert_eq!(
        Journal::open(&run_dir).read().unwrap(),
        vec![RunEvent::Usage {
            endpoint: "orchestrator".into(),
            tokens: Some(1),
            turns: None,
            tool_calls: None,
            cached_input_tokens: None,
            cache_creation_input_tokens: None,
        }],
        "the actual receipt is durable before display begins"
    );

    clock.advance(200);
    assert!(
        !cancellation.cancel_with_reason(saya_agent::CancellationReason::Budget(
            saya_agent::BudgetDimension::ElapsedTime
        ))
    );
    assert_eq!(
        cancellation.reason(),
        Some(saya_agent::CancellationReason::Budget(
            saya_agent::BudgetDimension::ReportedTokens
        )),
        "later elapsed expiry cannot replace the already observed token budget"
    );
    release_tx.send(()).unwrap();
    emit.await;
    assert_eq!(sink.state(), RunState::Executing);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn known_usage_budget_precedes_clock_uncertainty_and_keeps_its_diagnostic() {
    let root = temp_root("budget-clock-error");
    let (store, run_id) = seeded_store(&root, "budget-clock-error", RunStatus::Executing).await;
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).unwrap();
    let origin = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        + 60_000;
    let elapsed = ElapsedClock::arm(Duration::from_secs(5), origin, Instant::now()).unwrap();
    let cancellation = saya_agent::CancellationToken::default();
    let sink = EngineEventSink::new(
        run_id,
        RunState::Executing,
        Journal::open(&run_dir),
        store,
        SinkBudgets {
            wall_clock: None,
            token_ceiling: Some(1),
            download_budget: None,
            carried_usage: UsageTotals::default(),
        },
        Instant::now,
    )
    .with_elapsed_clock(elapsed)
    .with_cancellation(cancellation.clone());

    AgentEventSink::emit(
        &sink,
        AgentEvent::Usage {
            call: UsageCall::Answer,
            usage: TokenUsage::new(1, 0),
        },
    )
    .await;

    assert_eq!(
        cancellation.reason(),
        Some(saya_agent::CancellationReason::Budget(
            saya_agent::BudgetDimension::ReportedTokens
        )),
        "a known receipt is evaluated before uncertain elapsed time"
    );
    assert!(
        matches!(
            sink.take_diagnostic().as_deref(),
            Some(EngineSinkError::ElapsedClock { .. })
        ),
        "the clock uncertainty remains available beside the winning known-budget cause"
    );
    assert_eq!(
        Journal::open(&run_dir).read().unwrap(),
        vec![RunEvent::Usage {
            endpoint: "orchestrator".into(),
            tokens: Some(1),
            turns: None,
            tool_calls: None,
            cached_input_tokens: None,
            cache_creation_input_tokens: None,
        }]
    );
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn the_token_ceiling_pauses_the_run_once_it_is_spent() {
    let root = temp_root("tokens");
    let (store, run_id) = seeded_store(&root, "tokens", RunStatus::Approved).await;
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).unwrap();
    let sink = EngineEventSink::new(
        run_id.clone(),
        RunState::Approved,
        Journal::open(&run_dir),
        store.clone(),
        SinkBudgets {
            wall_clock: None,
            token_ceiling: Some(150),
            download_budget: None,
            carried_usage: UsageTotals::default(),
        },
        Instant::now,
    );
    sink.record(TransitionEvent::Begin).await.unwrap();
    assert_eq!(sink.state(), RunState::Executing);

    // Under the ceiling: counted, and the run keeps executing.
    AgentEventSink::emit(
        &sink,
        AgentEvent::Usage {
            call: UsageCall::Answer,
            usage: TokenUsage::new(60, 40),
        },
    )
    .await;
    assert_eq!(sink.state(), RunState::Executing, "100 of 150 is not spent");

    // Crossing it: input and output are summed, because the budget is what
    // the run costs and a ceiling on half of it bounds nothing in particular.
    AgentEventSink::emit(
        &sink,
        AgentEvent::Usage {
            call: UsageCall::Answer,
            usage: TokenUsage::new(40, 20),
        },
    )
    .await;
    assert_eq!(
        sink.state(),
        RunState::Paused,
        "160 tokens against a 150 ceiling must pause the run"
    );

    // The journal says *why*, so `run show` and a resume can both tell this
    // pause apart from a wall-clock one.
    let events = Journal::open(&run_dir).read().unwrap();
    assert!(
        events.iter().any(|event| matches!(
            event,
            RunEvent::Paused {
                reason: PauseReason::BudgetExhausted,
                ..
            }
        )),
        "the pause must name the budget: {events:?}"
    );

    let _ = fs::remove_dir_all(root);
}
