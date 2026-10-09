use super::*;

#[tokio::test]
async fn complete_journal_remains_authority_when_store_mirror_fails() {
    let run = approved_run("complete-mirror-failure").await;
    let store: Arc<dyn RunStore> = Arc::new(FailStatusMirror {
        inner: run.store.clone(),
        fail_status: RunStatus::Completed,
    });
    let provider = ScriptProvider::new(vec![Turn::Answer("done")]);
    let tools = vec![toolset(Arc::new(RecordingTools::default()), Vec::new())];
    let cancellation = CancellationToken::default();
    let driver = EpisodeDriver::new(
        EpisodeCollaborators {
            provider: &provider,
            approval: &AllowApproval,
            toolsets: &tools,
            cancellation: cancellation.clone(),
        },
        EpisodeRun {
            run_id: run.run_id.clone(),
            store: store.clone(),
            journal: Journal::open(&run.run_dir),
        },
        EpisodeRequest {
            model: "mock-model".into(),
            profile_names: Vec::new(),
            memory_allows_candidate_writes: false,
        },
        bounds(),
    );
    let sink = EngineEventSink::new(
        run.run_id.clone(),
        RunState::Approved,
        Journal::open(&run.run_dir),
        store,
        SinkBudgets {
            wall_clock: None,
            token_ceiling: None,
            download_budget: None,
            carried_usage: UsageTotals::default(),
        },
        std::time::Instant::now,
    )
    .with_cancellation(cancellation);
    let plan = RunPlan::new(vec![step(
        "complete with mirror fault",
        Capabilities::default(),
    )])
    .unwrap();

    let error = driver
        .run_step(&sink, &plan, 0, &run.workspace())
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        EpisodeError::Transition {
            source: saya_harness::engine::EngineSinkError::Store {
                source: StoreError::Unavailable
            }
        }
    ));
    assert_eq!(sink.state(), RunState::Completed);
    assert_eq!(
        Journal::open(&run.run_dir).read().unwrap().last(),
        Some(&RunEvent::Completed),
        "the appended terminal journal event remains authoritative"
    );
    let _ = fs::remove_dir_all(run.root);
}

#[tokio::test]
async fn user_stop_after_final_failed_mirror_wins_terminal_handoff() {
    let run = approved_run("failed-mirror-stop").await;
    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let store: Arc<dyn RunStore> = Arc::new(HoldFinalFailedMirror {
        inner: run.store.clone(),
        failures: AtomicUsize::new(0),
        entered: std::sync::Mutex::new(Some(entered_tx)),
        release: std::sync::Mutex::new(Some(release_rx)),
    });
    let provider = ScriptProvider::new(
        std::iter::repeat_with(|| Turn::Fail)
            .take(saya_harness::engine::MAX_EPISODE_ATTEMPTS)
            .collect(),
    );
    let executor = Arc::new(RecordingTools::default());
    let tools = vec![toolset(executor.clone(), Vec::new())];
    let cancellation = CancellationToken::default();
    let driver = EpisodeDriver::new(
        EpisodeCollaborators {
            provider: &provider,
            approval: &AllowApproval,
            toolsets: &tools,
            cancellation: cancellation.clone(),
        },
        EpisodeRun {
            run_id: run.run_id.clone(),
            store: store.clone(),
            journal: Journal::open(&run.run_dir),
        },
        EpisodeRequest {
            model: "mock-model".into(),
            profile_names: Vec::new(),
            memory_allows_candidate_writes: false,
        },
        bounds(),
    );
    let sink = EngineEventSink::new(
        run.run_id.clone(),
        RunState::Approved,
        Journal::open(&run.run_dir),
        store,
        SinkBudgets {
            wall_clock: None,
            token_ceiling: None,
            download_budget: None,
            carried_usage: UsageTotals::default(),
        },
        std::time::Instant::now,
    )
    .with_cancellation(cancellation.clone());
    let plan = RunPlan::new(vec![step("fail before settle", Capabilities::default())]).unwrap();
    let workspace = run.workspace();
    let active = driver.run_step(&sink, &plan, 0, &workspace);
    tokio::pin!(active);

    tokio::time::timeout(Duration::from_secs(3), async {
        tokio::select! {
            entered = entered_rx => entered.unwrap(),
            result = &mut active => panic!("final failed-step mirror did not hold: {result:?}"),
        }
    })
    .await
    .expect("final failed-step store mirror starts");
    assert!(cancellation.cancel_with_reason(CancellationReason::User));
    assert!(
        !Journal::open(&run.run_dir)
            .read()
            .unwrap()
            .iter()
            .any(|event| matches!(event, RunEvent::Paused { .. } | RunEvent::Cancelled)),
        "the terminal decision waits for the pending mark_failed mirror"
    );

    release_tx.send(()).unwrap();
    let error = active.await.unwrap_err();
    assert!(matches!(error, EpisodeError::Cancelled { step: 0 }));
    assert_eq!(
        Journal::open(&run.run_dir).read().unwrap().last(),
        Some(&RunEvent::Cancelled),
        "the stop accepted before the terminal append owns the single lifecycle event"
    );
    assert_eq!(
        Journal::open(&run.run_dir)
            .read()
            .unwrap()
            .iter()
            .filter(|event| matches!(event, RunEvent::Paused { .. } | RunEvent::Cancelled))
            .count(),
        1
    );
    assert_eq!(sink.state(), RunState::Cancelled);
    let _ = fs::remove_dir_all(run.root);
}
