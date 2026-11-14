use super::*;

#[tokio::test]
async fn the_wall_clock_deadline_trips_a_pause_per_tick() {
    let clock = SimClock::new();
    let root = temp_root("deadline");
    let (store, run_id) = seeded_store(&root, "deadline", RunStatus::Approved).await;
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).unwrap();
    let clock_for_sink = clock.clone();
    let sink = EngineEventSink::new(
        run_id.clone(),
        RunState::Approved,
        Journal::open(&run_dir),
        store.clone(),
        SinkBudgets {
            wall_clock: Some(Duration::from_millis(100)),
            token_ceiling: None,
            download_budget: None,
            carried_usage: UsageTotals::default(),
        },
        move || clock_for_sink.clock(),
    );

    // Executing, before the deadline: a tick is a no-op — no pause, no
    // journal line.
    sink.record(TransitionEvent::Begin).await.unwrap();
    assert_eq!(sink.state(), RunState::Executing);
    AgentEventSink::emit(
        &sink,
        AgentEvent::Usage {
            call: UsageCall::Answer,
            usage: TokenUsage::new(1, 1),
        },
    )
    .await;
    assert_eq!(sink.state(), RunState::Executing);
    let events = Journal::open(&run_dir).read().unwrap();
    assert!(
        matches!(events.as_slice(), [RunEvent::Usage { .. }]),
        "a tick before the deadline must not pause; the call's usage is \
         journaled as the durable record, nothing more: {events:?}"
    );

    // Past the deadline, the very next tick pauses the run — once, with the
    // wall-clock reason, in the journal and in the store.
    clock.advance(150);
    AgentEventSink::emit(
        &sink,
        AgentEvent::Usage {
            call: UsageCall::Answer,
            usage: TokenUsage::new(1, 1),
        },
    )
    .await;
    assert_eq!(sink.state(), RunState::Paused);
    let events = Journal::open(&run_dir).read().unwrap();
    assert_eq!(
        events,
        vec![
            RunEvent::Usage {
                endpoint: "orchestrator".into(),
                tokens: Some(2),
                turns: None,
                tool_calls: None,
                cached_input_tokens: None,
                cache_creation_input_tokens: None,
            },
            RunEvent::Usage {
                endpoint: "orchestrator".into(),
                tokens: Some(2),
                turns: None,
                tool_calls: None,
                cached_input_tokens: None,
                cache_creation_input_tokens: None,
            },
            RunEvent::Paused {
                reason: PauseReason::WallClockExceeded
            },
        ],
        "each call's usage is journaled, and the deadline pause exactly once, with its reason"
    );
    let record = store.get_run(&run_id).await.unwrap().unwrap();
    assert_eq!(
        record.status,
        RunStatus::Paused,
        "the pause must be mirrored to the store"
    );

    // A paused run does not re-pause: further ticks never write a second
    // pause — the only thing a later usage event adds is its own record.
    clock.advance(10_000);
    AgentEventSink::emit(
        &sink,
        AgentEvent::Usage {
            call: UsageCall::Answer,
            usage: TokenUsage::new(1, 1),
        },
    )
    .await;
    let events = Journal::open(&run_dir).read().unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, RunEvent::Paused { .. }))
            .count(),
        1,
        "a paused run never re-pauses: {events:?}"
    );

    let _ = fs::remove_dir_all(root);
}
