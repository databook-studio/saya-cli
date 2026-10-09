use super::*;

/// The trip latch pauses the run on the first tick after the trip, with the
/// same reason the token ceiling uses. The sink holds a *clone* of the
/// wallet the fetch-capable executors hold — clone-shares-state — so the
/// test trips it through a second clone exactly the way a refused download
/// inside a step would.
#[tokio::test]
async fn the_download_budget_s_trip_latch_pauses_the_run_once_it_trips() {
    let root = temp_root("download-latch");
    let (store, run_id) = seeded_store(&root, "download-latch", RunStatus::Approved).await;
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).unwrap();
    let budget = DownloadBudget::new(100);
    let sink = EngineEventSink::new(
        run_id.clone(),
        RunState::Approved,
        Journal::open(&run_dir),
        store.clone(),
        SinkBudgets {
            wall_clock: None,
            token_ceiling: None,
            download_budget: Some(budget.clone()),
            carried_usage: UsageTotals::default(),
        },
        Instant::now,
    );
    sink.record(TransitionEvent::Begin).await.unwrap();
    assert_eq!(sink.state(), RunState::Executing);

    // A run consuming under its budget with nothing refused keeps executing.
    assert!(budget.claim(60));
    AgentEventSink::emit(
        &sink,
        AgentEvent::Usage {
            call: UsageCall::Answer,
            usage: TokenUsage::new(1, 1),
        },
    )
    .await;
    assert_eq!(sink.state(), RunState::Executing, "no refusal, no pause");

    // The trip: a claim refused through the executor's clone of the same
    // wallet — the typed `BudgetExhausted` the tool reports the model —
    // also reaches the sink, and the next tick pauses the run.
    assert!(!budget.claim(50), "the refusing claim");
    assert!(budget.tripped(), "the refusal is the recorded event");

    AgentEventSink::emit(
        &sink,
        AgentEvent::Usage {
            call: UsageCall::Answer,
            usage: TokenUsage::new(1, 1),
        },
    )
    .await;
    assert_eq!(
        sink.state(),
        RunState::Paused,
        "a tripped budget pauses the run on the first tick after the trip"
    );
    assert!(
        Journal::open(&run_dir)
            .read()
            .unwrap()
            .iter()
            .any(|event| matches!(
                event,
                RunEvent::Paused {
                    reason: PauseReason::BudgetExhausted
                }
            )),
        "the pause must name the budget — the shared vocabulary, no new reason"
    );
    let record = store.get_run(&run_id).await.unwrap().unwrap();
    assert_eq!(record.status, RunStatus::Paused);

    let _ = fs::remove_dir_all(root);
}

/// The non-regression the latch exists for: a run that downloads *exactly*
/// its budget with nothing refused never pauses. A `consumed >= limit`
/// threshold would stop this run; the latch — the recorded refusal — does
/// not fire.
#[tokio::test]
async fn a_wallet_consumed_exactly_to_its_limit_without_a_refusal_never_pauses() {
    let root = temp_root("download-exact");
    let (store, run_id) = seeded_store(&root, "download-exact", RunStatus::Approved).await;
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).unwrap();
    let budget = DownloadBudget::new(100);
    assert!(
        budget.claim(60) && budget.claim(40),
        "exact fill, none refused"
    );
    assert_eq!(budget.consumed(), 100);
    assert!(!budget.tripped());
    let sink = EngineEventSink::new(
        run_id,
        RunState::Approved,
        Journal::open(&run_dir),
        store,
        SinkBudgets {
            wall_clock: None,
            token_ceiling: None,
            download_budget: Some(budget),
            carried_usage: UsageTotals::default(),
        },
        Instant::now,
    );
    sink.record(TransitionEvent::Begin).await.unwrap();
    AgentEventSink::emit(
        &sink,
        AgentEvent::Usage {
            call: UsageCall::Answer,
            usage: TokenUsage::new(1, 1),
        },
    )
    .await;
    assert_eq!(
        sink.state(),
        RunState::Executing,
        "an exact fill with nothing refused is not a budget exhaustion"
    );
    let _ = fs::remove_dir_all(root);
}

/// The wallet the run did not approve is `None`, and the check is inert —
/// the `token_ceiling: None` pattern. No run that never approved fetch may
/// pause on a download budget that does not exist.
#[tokio::test]
async fn no_armed_download_budget_means_no_download_pause() {
    let root = temp_root("download-none");
    let (store, run_id) = seeded_store(&root, "download-none", RunStatus::Approved).await;
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
            usage: TokenUsage::new(1, 1),
        },
    )
    .await;
    assert_eq!(sink.state(), RunState::Executing);
    let _ = fs::remove_dir_all(root);
}
