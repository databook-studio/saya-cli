use super::*;

#[tokio::test]
async fn eventless_provider_deadline_cancels_local_attempt_then_pauses_once() {
    with_ceiling_beyond_setup(
        &[
            Duration::from_millis(250),
            Duration::from_secs(1),
            Duration::from_secs(4),
            Duration::from_secs(16),
        ],
        eventless_provider_attempt,
    )
    .await;
}

async fn eventless_provider_attempt(ceiling: Duration) -> bool {
    let run = approved_run("eventless-deadline").await;
    let (started_tx, started_rx) = oneshot::channel();
    let (abandoned_tx, abandoned_rx) = oneshot::channel();
    let provider = PendingProvider {
        started: Mutex::new(Some(started_tx)),
        abandoned: Mutex::new(Some(abandoned_tx)),
    };
    let tools = vec![toolset(Arc::new(RecordingTools::default()), Vec::new())];
    let cancellation = CancellationToken::default();
    let driver = deadline_driver(&run, &provider, &tools, cancellation.clone());
    let sink = deadline_sink(&run, cancellation.clone(), ceiling);
    let workspace = run.workspace();
    let plan = RunPlan::new(vec![step("wait for provider", Capabilities::default())]).unwrap();
    let active = driver.run_step(&sink, &plan, 0, &workspace);
    tokio::pin!(active);

    let started = tokio::time::timeout(ceiling + HANG_BOUND, async {
        tokio::select! {
            started = started_rx => {
                started.unwrap();
                true
            }
            result = &mut active => {
                assert!(
                    is_elapsed_time_stop(&result),
                    "episode finished before provider start: {result:?}"
                );
                false
            }
        }
    })
    .await
    .expect("provider setup starts");
    if !started {
        let _ = fs::remove_dir_all(run.root);
        return false;
    }
    tokio::time::timeout(ceiling + HANG_BOUND, async {
        tokio::select! {
            _ = cancellation.cancelled() => {},
            result = &mut active => panic!("episode returned without a stop reason: {result:?}"),
        }
    })
    .await
    .expect("elapsed timer stops an eventless provider attempt");
    assert_eq!(
        cancellation.reason(),
        Some(CancellationReason::Budget(BudgetDimension::ElapsedTime))
    );
    assert!(!cancellation.cancel_with_reason(CancellationReason::User));
    assert_eq!(
        cancellation.reason(),
        Some(CancellationReason::Budget(BudgetDimension::ElapsedTime)),
        "a later user request cannot rewrite the timer's first stop cause"
    );
    tokio::time::timeout(HANG_BOUND, abandoned_rx)
        .await
        .expect("local provider stream future is abandoned")
        .unwrap();

    let error = active.await.unwrap_err();
    assert!(matches!(error, EpisodeError::BudgetStopped { step: 0, .. }));
    assert_eq!(sink.state(), RunState::Paused);
    let events = Journal::open(&run.run_dir).read().unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, RunEvent::Paused { .. }))
            .count(),
        1
    );
    assert!(events.contains(&RunEvent::Paused {
        reason: PauseReason::WallClockExceeded,
    }));
    let _ = fs::remove_dir_all(run.root);
    true
}

#[tokio::test]
async fn held_tool_keeps_lifecycle_open_until_the_cancelled_source_returns() {
    with_ceiling_beyond_setup(
        &[
            Duration::from_secs(2),
            Duration::from_secs(6),
            Duration::from_secs(20),
        ],
        held_tool_attempt,
    )
    .await;
}

async fn held_tool_attempt(ceiling: Duration) -> bool {
    let run = approved_run("held-tool-deadline").await;
    let provider = ScriptProvider::new(vec![Turn::Tools(vec![call("probe")])]);
    let (started_tx, started_rx) = oneshot::channel();
    let (stopped_tx, stopped_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let executor = Arc::new(HeldTool {
        started: Mutex::new(Some(started_tx)),
        stopped: Mutex::new(Some(stopped_tx)),
        release: Mutex::new(Some(release_rx)),
    });
    let tools = vec![toolset(
        executor,
        vec![def("probe", LocalStateEffect::None)],
    )];
    let cancellation = CancellationToken::default();
    let driver = deadline_driver(&run, &provider, &tools, cancellation.clone());
    let sink = deadline_sink(&run, cancellation.clone(), ceiling);
    let workspace = run.workspace();
    let plan = RunPlan::new(vec![step("hold tool", Capabilities::default())]).unwrap();
    let active = driver.run_step(&sink, &plan, 0, &workspace);
    tokio::pin!(active);

    let started = tokio::time::timeout(ceiling + HANG_BOUND, async {
        tokio::select! {
            started = started_rx => {
                started.unwrap();
                true
            }
            result = &mut active => {
                assert!(
                    is_elapsed_time_stop(&result),
                    "episode finished before tool start: {result:?}"
                );
                false
            }
        }
    })
    .await
    .expect("tool source starts");
    if !started {
        let _ = fs::remove_dir_all(run.root);
        return false;
    }
    tokio::time::timeout(ceiling + HANG_BOUND, async {
        tokio::select! {
            stopped = stopped_rx => stopped.unwrap(),
            result = &mut active => panic!("episode settled before source release: {result:?}"),
        }
    })
    .await
    .expect("tool receives the same deadline token");
    assert_eq!(sink.state(), RunState::Executing);
    assert!(
        !Journal::open(&run.run_dir)
            .read()
            .unwrap()
            .iter()
            .any(|event| matches!(event, RunEvent::Paused { .. })),
        "the run must remain active while the source owns its pending future"
    );

    release_tx.send(()).unwrap();
    let error = active.await.unwrap_err();
    assert!(matches!(error, EpisodeError::BudgetStopped { step: 0, .. }));
    assert_eq!(sink.state(), RunState::Paused);
    assert_eq!(
        Journal::open(&run.run_dir)
            .read()
            .unwrap()
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    RunEvent::Paused {
                        reason: PauseReason::WallClockExceeded
                    }
                )
            })
            .count(),
        1
    );
    let _ = fs::remove_dir_all(run.root);
    true
}

#[tokio::test]
async fn user_stop_settles_the_same_source_and_keeps_first_cause() {
    let run = approved_run("held-tool-user-stop").await;
    let provider = ScriptProvider::new(vec![Turn::Tools(vec![call("probe")])]);
    let (started_tx, started_rx) = oneshot::channel();
    let (stopped_tx, stopped_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let executor = Arc::new(HeldTool {
        started: Mutex::new(Some(started_tx)),
        stopped: Mutex::new(Some(stopped_tx)),
        release: Mutex::new(Some(release_rx)),
    });
    let tools = vec![toolset(
        executor,
        vec![def("probe", LocalStateEffect::None)],
    )];
    let cancellation = CancellationToken::default();
    let driver = deadline_driver(&run, &provider, &tools, cancellation.clone());
    // The elapsed timer is not under test here, so its ceiling is far beyond
    // the run: the user stop is the only stop that can happen.
    let sink = deadline_sink(&run, cancellation.clone(), Duration::from_secs(60));
    let workspace = run.workspace();
    let plan = RunPlan::new(vec![step("hold tool", Capabilities::default())]).unwrap();
    let active = driver.run_step(&sink, &plan, 0, &workspace);
    tokio::pin!(active);

    tokio::time::timeout(HANG_BOUND, async {
        tokio::select! {
            started = started_rx => started.unwrap(),
            result = &mut active => panic!("episode finished before tool start: {result:?}"),
        }
    })
    .await
    .expect("tool source starts");
    assert!(cancellation.cancel_with_reason(CancellationReason::User));
    assert!(
        !cancellation.cancel_with_reason(CancellationReason::Budget(BudgetDimension::ElapsedTime,))
    );
    tokio::time::timeout(HANG_BOUND, async {
        tokio::select! {
            stopped = stopped_rx => stopped.unwrap(),
            result = &mut active => panic!("episode settled before user source release: {result:?}"),
        }
    })
    .await
    .expect("tool source observes user stop");
    assert_eq!(sink.state(), RunState::Executing);
    assert!(
        !Journal::open(&run.run_dir)
            .read()
            .unwrap()
            .iter()
            .any(|event| matches!(event, RunEvent::Cancelled)),
        "cancellation is journaled only after the active source returns"
    );

    release_tx.send(()).unwrap();
    let error = active.await.unwrap_err();
    assert!(matches!(error, EpisodeError::Cancelled { step: 0 }));
    assert_eq!(sink.state(), RunState::Cancelled);
    assert_eq!(
        Journal::open(&run.run_dir)
            .read()
            .unwrap()
            .iter()
            .filter(|event| matches!(event, RunEvent::Cancelled))
            .count(),
        1
    );
    let _ = fs::remove_dir_all(run.root);
}
