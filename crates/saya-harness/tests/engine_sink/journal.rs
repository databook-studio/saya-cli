use super::*;

#[tokio::test]
async fn the_sink_journals_the_wallet_s_level_as_the_spend_grows() {
    let root = temp_root("download-journal");
    let (store, run_id) = seeded_store(&root, "download-journal", RunStatus::Approved).await;
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).unwrap();
    let budget = DownloadBudget::new(100);
    let sink = EngineEventSink::new(
        run_id,
        RunState::Approved,
        Journal::open(&run_dir),
        store,
        SinkBudgets {
            wall_clock: None,
            token_ceiling: None,
            download_budget: Some(budget.clone()),
            carried_usage: UsageTotals::default(),
        },
        Instant::now,
    );
    sink.record(TransitionEvent::Begin).await.unwrap();
    let journal = Journal::open(&run_dir);

    // No download yet: no spend, no level recorded.
    AgentEventSink::emit(&sink, an_event()).await;
    assert!(
        journaled_levels(&journal).is_empty(),
        "an emit with no download growth must record no level"
    );

    // The claims a download made are journaled at the next tick — exactly
    // once, though every event emission is a tick.
    assert!(budget.claim(60));
    AgentEventSink::emit(&sink, an_event()).await;
    AgentEventSink::emit(&sink, an_event()).await;
    assert_eq!(
        journaled_levels(&journal),
        vec![60],
        "the level journals once per growth, not once per emit"
    );

    // The next growth records the level it reached: 100, the wallet's whole
    // spend so far — the figure a resume carries.
    assert!(budget.claim(40));
    AgentEventSink::emit(&sink, an_event()).await;
    assert_eq!(journaled_levels(&journal), vec![60, 100]);

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn a_seeded_sink_never_re_journals_the_level_its_record_holds() {
    let root = temp_root("download-seeded");
    let (store, run_id) = seeded_store(&root, "download-seeded", RunStatus::Approved).await;
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).unwrap();
    let journal = Journal::open(&run_dir);
    journal
        .append(&RunEvent::DownloadedBytes { bytes: 60 })
        .unwrap();
    let budget = DownloadBudget::new(100);
    budget.carry(60);
    let sink = EngineEventSink::new(
        run_id,
        RunState::Approved,
        Journal::open(&run_dir),
        store,
        SinkBudgets {
            wall_clock: None,
            token_ceiling: None,
            download_budget: Some(budget.clone()),
            carried_usage: UsageTotals::default(),
        },
        Instant::now,
    );
    sink.record(TransitionEvent::Begin).await.unwrap();

    // The carried spend is not this invocation's: emitting with nothing
    // claimed must record nothing.
    AgentEventSink::emit(&sink, an_event()).await;
    AgentEventSink::emit(&sink, an_event()).await;
    assert_eq!(
        journaled_levels(&journal),
        vec![60],
        "the record's level must not be re-journaled as fresh spend"
    );

    // This invocation's own claims record their growth past the carried
    // level — the level it reached, not the bytes it claimed.
    assert!(budget.claim(40));
    AgentEventSink::emit(&sink, an_event()).await;
    assert_eq!(journaled_levels(&journal), vec![60, 100]);

    // And the record carries exactly what the wallet holds: the max of the
    // recorded levels, the figure the next resume seeds.
    assert_eq!(budget.consumed(), 100);
    assert_eq!(
        DownloadBudget::carried_from_journal(&journal.read().unwrap()),
        100
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn the_recorded_spend_precedes_the_pause_that_stopped_it() {
    let root = temp_root("download-order");
    let (store, run_id) = seeded_store(&root, "download-order", RunStatus::Approved).await;
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).unwrap();
    let budget = DownloadBudget::new(100);
    let sink = EngineEventSink::new(
        run_id,
        RunState::Approved,
        Journal::open(&run_dir),
        store,
        SinkBudgets {
            wall_clock: None,
            token_ceiling: None,
            download_budget: Some(budget.clone()),
            carried_usage: UsageTotals::default(),
        },
        Instant::now,
    );
    sink.record(TransitionEvent::Begin).await.unwrap();
    let journal = Journal::open(&run_dir);

    // The download's claims land, the refusal trips the latch, and the
    // next tick — the same one that journals the level — pauses the run.
    assert!(budget.claim(60));
    assert!(!budget.claim(50), "the refusing claim");
    AgentEventSink::emit(&sink, an_event()).await;
    assert_eq!(sink.state(), RunState::Paused);
    assert_eq!(
        journaled_levels(&journal),
        vec![60],
        "the spend the record holds is the level at the trip"
    );
    let events = journal.read().unwrap();
    let pause_at = events
        .iter()
        .position(|event| matches!(event, RunEvent::Paused { .. }))
        .expect("the run must be paused in the journal");
    let level_at = events
        .iter()
        .position(|event| matches!(event, RunEvent::DownloadedBytes { .. }))
        .expect("the level must be recorded");
    assert!(
        level_at < pause_at,
        "the spend must be journaled before the pause that stopped it"
    );

    let _ = fs::remove_dir_all(root);
}
