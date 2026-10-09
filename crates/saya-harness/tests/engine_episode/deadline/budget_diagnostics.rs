use super::*;

#[tokio::test]
async fn reported_token_budget_pause_mirror_failure_keeps_clock_diagnostic() {
    let run = approved_run("budget-pause-mirror-failure").await;
    run.store
        .set_run_status(&run.run_id, RunStatus::Executing, None)
        .await
        .unwrap();
    let store: Arc<dyn RunStore> = Arc::new(FailStatusMirror {
        inner: run.store.clone(),
        fail_status: RunStatus::Paused,
    });
    let provider = ScriptProvider::new(Vec::new());
    let tools = vec![toolset(Arc::new(RecordingTools::default()), Vec::new())];
    let cancellation = CancellationToken::default();
    let driver = deadline_driver(&run, &provider, &tools, cancellation.clone());
    let future_origin = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        + 60_000;
    let elapsed = ElapsedClock::arm(
        Duration::from_secs(5),
        future_origin,
        std::time::Instant::now(),
    )
    .unwrap();
    let sink = EngineEventSink::new(
        run.run_id.clone(),
        RunState::Executing,
        Journal::open(&run.run_dir),
        store,
        SinkBudgets {
            wall_clock: None,
            token_ceiling: Some(1),
            download_budget: None,
            carried_usage: UsageTotals::default(),
        },
        std::time::Instant::now,
    )
    .with_elapsed_clock(elapsed)
    .with_cancellation(cancellation.clone());

    saya_agent::AgentEventSink::emit(
        &sink,
        saya_agent::AgentEvent::Usage {
            call: saya_agent::UsageCall::Answer,
            usage: TokenUsage::new(1, 0),
        },
    )
    .await;
    assert_eq!(
        cancellation.reason(),
        Some(CancellationReason::Budget(BudgetDimension::ReportedTokens))
    );
    let plan = RunPlan::new(vec![step("settle budget receipt", Capabilities::default())]).unwrap();

    let error = driver
        .run_step(&sink, &plan, 0, &run.workspace())
        .await
        .unwrap_err();

    assert!(
        matches!(
            &error,
            EpisodeError::BudgetPersistenceFailed {
                reason: PauseReason::BudgetExhausted,
                diagnostic,
                transition,
                ..
            } if matches!(
                diagnostic.as_ref(),
                saya_harness::engine::EngineSinkError::ElapsedClock { .. }
            ) && matches!(
                transition.as_ref(),
                saya_harness::engine::EngineSinkError::Store {
                    source: StoreError::Unavailable
                }
            )
        ),
        "the budget reason and both ordered diagnostics must survive: {error:?}"
    );
    assert_eq!(
        cancellation.reason(),
        Some(CancellationReason::Budget(BudgetDimension::ReportedTokens))
    );
    assert_eq!(sink.state(), RunState::Paused);
    let events = Journal::open(&run.run_dir).read().unwrap();
    assert!(events.iter().any(|event| matches!(
        event,
        RunEvent::Usage {
            tokens: Some(1),
            ..
        }
    )));
    assert_eq!(
        events.last(),
        Some(&RunEvent::Paused {
            reason: PauseReason::BudgetExhausted,
        })
    );
    let _ = fs::remove_dir_all(run.root);
}
