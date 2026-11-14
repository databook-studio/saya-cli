use super::*;

#[tokio::test]
async fn the_usage_a_budget_pause_shows_agrees_with_the_ceiling_that_stopped_it() {
    let root = temp_root("tokens-display");
    let (store, run_id) = seeded_store(&root, "tokens-display", RunStatus::Approved).await;
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).unwrap();
    let sink = EngineEventSink::new(
        run_id,
        RunState::Approved,
        Journal::open(&run_dir),
        store,
        SinkBudgets {
            wall_clock: None,
            token_ceiling: Some(150),
            download_budget: None,
            carried_usage: UsageTotals::default(),
        },
        Instant::now,
    );
    sink.record(TransitionEvent::Begin).await.unwrap();

    // 100 of 150, then the crossing call — the same shape the pause test
    // above drives, so both readings of the same run must agree.
    AgentEventSink::emit(
        &sink,
        AgentEvent::Usage {
            call: UsageCall::Answer,
            usage: TokenUsage::new(60, 40),
        },
    )
    .await;
    AgentEventSink::emit(
        &sink,
        AgentEvent::Usage {
            call: UsageCall::Answer,
            usage: TokenUsage::new(40, 20),
        },
    )
    .await;
    assert_eq!(sink.state(), RunState::Paused);

    let events = Journal::open(&run_dir).read().unwrap();
    let journaled: u64 = events
        .iter()
        .map(|event| match event {
            RunEvent::Usage {
                tokens: Some(tokens),
                ..
            } => *tokens,
            _ => 0,
        })
        .sum();
    assert_eq!(
        journaled, 160,
        "the journal must record what the run spent: {events:?}"
    );
    assert!(
        journaled >= 150,
        "usage shown for a budget pause must agree with the ceiling that stopped it"
    );
    let totals = sink.usage();
    assert_eq!(
        totals.input_tokens.saturating_add(totals.output_tokens),
        journaled,
        "the ceiling's arithmetic and the durable record must agree"
    );
    for event in &events {
        if let RunEvent::Usage { endpoint, .. } = event {
            assert_eq!(
                endpoint, "orchestrator",
                "every episode calls the one endpoint the ceiling sums: {events:?}"
            );
        }
    }
    let pause_index = events
        .iter()
        .position(|event| {
            matches!(
                event,
                RunEvent::Paused {
                    reason: PauseReason::BudgetExhausted
                }
            )
        })
        .expect("the pause must be journaled");
    let usage_index = events
        .iter()
        .position(|event| matches!(event, RunEvent::Usage { .. }))
        .expect("the usage must be journaled");
    assert!(
        usage_index < pause_index,
        "the usage that tripped the ceiling precedes the pause naming it: {events:?}"
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn no_declared_ceiling_means_no_token_pause() {
    let root = temp_root("notokens");
    let (store, run_id) = seeded_store(&root, "notokens", RunStatus::Approved).await;
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).unwrap();
    let sink = EngineEventSink::new(
        run_id,
        RunState::Approved,
        Journal::open(&run_dir),
        store,
        SinkBudgets {
            wall_clock: None,
            token_ceiling: None,
            download_budget: None,
            carried_usage: UsageTotals::default(),
        },
        Instant::now,
    );
    sink.record(TransitionEvent::Begin).await.unwrap();
    AgentEventSink::emit(
        &sink,
        AgentEvent::Usage {
            call: UsageCall::Answer,
            usage: TokenUsage::new(10_000_000, 10_000_000),
        },
    )
    .await;
    assert_eq!(sink.state(), RunState::Executing);
    let _ = fs::remove_dir_all(root);
}

/// The ceiling binds the run's whole spend, not each invocation's: a sink
/// seeded from its journal — the seeding a resume does — pauses on the next
/// tick even though the spend this invocation has made is far under the
/// ceiling. Without the seed, a resumed invocation re-armed the ceiling in
/// full and could spend it again; this is the sink-side form of the defect
/// the resume tests drive end to end.
#[tokio::test]
async fn the_ceiling_binds_the_run_s_whole_spend_not_each_invocation_s() {
    let root = temp_root("seeded-ceiling");
    let (store, run_id) = seeded_store(&root, "seeded-ceiling", RunStatus::Executing).await;
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).unwrap();
    // The journal a ceiling-paused run leaves: each call's usage, then the
    // pause — 160 tokens spent against the 150 ceiling.
    let journaled = vec![journaled_usage(100), journaled_usage(60)];
    let journal = Journal::open(&run_dir);
    for event in &journaled {
        journal.append(event).unwrap();
    }
    let sink = EngineEventSink::new(
        run_id,
        RunState::Executing,
        journal,
        store,
        SinkBudgets {
            wall_clock: None,
            token_ceiling: Some(150),
            download_budget: None,
            carried_usage: UsageTotals::from_journal(&journaled),
        },
        Instant::now,
    );
    assert_eq!(
        sink.usage().carried_tokens,
        160,
        "the journal seeds the spend the run already made"
    );

    // One token this invocation — far under a fresh 150 — and the run
    // pauses: the ceiling compares the run's 161, not the invocation's 1.
    AgentEventSink::emit(
        &sink,
        AgentEvent::Usage {
            call: UsageCall::Answer,
            usage: TokenUsage::new(1, 0),
        },
    )
    .await;
    assert_eq!(
        sink.state(),
        RunState::Paused,
        "161 tokens against a 150 ceiling must pause the run, though the \
         invocation itself spent one"
    );
    assert!(matches!(
        Journal::open(&run_dir).read().unwrap().last(),
        Some(RunEvent::Paused {
            reason: PauseReason::BudgetExhausted
        })
    ));

    let _ = fs::remove_dir_all(root);
}
