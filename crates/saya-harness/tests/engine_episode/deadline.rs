use super::*;
use saya_agent::{BudgetDimension, CancellationReason, ProviderStream};
use saya_harness::engine::ElapsedClock;
use std::time::Duration;
use tokio::sync::oneshot;

struct PendingProvider {
    started: Mutex<Option<oneshot::Sender<()>>>,
    abandoned: Mutex<Option<oneshot::Sender<()>>>,
}

struct AbandonSignal(Option<oneshot::Sender<()>>);

impl Drop for AbandonSignal {
    fn drop(&mut self) {
        if let Some(signal) = self.0.take() {
            let _ = signal.send(());
        }
    }
}

#[async_trait]
impl ChatProvider for PendingProvider {
    fn name(&self) -> &str {
        "pending"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("the test uses the provider stream owner")
    }

    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        let _ = self.started.lock().unwrap().take().unwrap().send(());
        let _abandon = AbandonSignal(self.abandoned.lock().unwrap().take());
        std::future::pending().await
    }
}

struct HeldTool {
    started: Mutex<Option<oneshot::Sender<()>>>,
    stopped: Mutex<Option<oneshot::Sender<()>>>,
    release: Mutex<Option<oneshot::Receiver<()>>>,
}

#[async_trait]
impl ToolExecutor for HeldTool {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        unreachable!("the agent must pass its cancellation context")
    }

    async fn execute_with_context(
        &self,
        _: &str,
        _: serde_json::Value,
        context: saya_agent::ToolExecutionContext,
    ) -> Result<serde_json::Value, ToolError> {
        let _ = self.started.lock().unwrap().take().unwrap().send(());
        context.cancellation.cancelled().await;
        let _ = self.stopped.lock().unwrap().take().unwrap().send(());
        let release = self.release.lock().unwrap().take().unwrap();
        release.await.unwrap();
        Ok(serde_json::json!({"settled": true}))
    }
}

fn deadline_sink(
    run: &ApprovedRun,
    cancellation: CancellationToken,
    ceiling: Duration,
) -> EngineEventSink {
    EngineEventSink::new(
        run.run_id.clone(),
        RunState::Approved,
        Journal::open(&run.run_dir),
        run.store.clone(),
        SinkBudgets {
            wall_clock: None,
            token_ceiling: None,
            download_budget: None,
            carried_usage: UsageTotals::default(),
        },
        std::time::Instant::now,
    )
    .with_elapsed_clock(ElapsedClock::arm_now(ceiling).unwrap())
    .with_cancellation(cancellation)
}

fn deadline_driver<'a>(
    run: &ApprovedRun,
    provider: &'a dyn ChatProvider,
    tools: &'a [StepToolset],
    cancellation: CancellationToken,
) -> EpisodeDriver<'a> {
    EpisodeDriver::new(
        EpisodeCollaborators {
            provider,
            approval: &AllowApproval,
            toolsets: tools,
            cancellation,
        },
        EpisodeRun {
            run_id: run.run_id.clone(),
            store: run.store.clone(),
            journal: Journal::open(&run.run_dir),
        },
        EpisodeRequest {
            model: "mock-model".into(),
            profile_names: Vec::new(),
            memory_allows_candidate_writes: false,
        },
        bounds(),
    )
}

#[tokio::test]
async fn eventless_provider_deadline_cancels_local_attempt_then_pauses_once() {
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
    let sink = deadline_sink(&run, cancellation.clone(), Duration::from_millis(250));
    let workspace = run.workspace();
    let plan = RunPlan::new(vec![step("wait for provider", Capabilities::default())]).unwrap();
    let active = driver.run_step(&sink, &plan, 0, &workspace);
    tokio::pin!(active);

    tokio::time::timeout(Duration::from_secs(1), async {
        tokio::select! {
            started = started_rx => started.unwrap(),
            result = &mut active => panic!("episode finished before provider start: {result:?}"),
        }
    })
    .await
    .expect("provider setup starts");
    tokio::time::timeout(Duration::from_secs(1), async {
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
    tokio::time::timeout(Duration::from_secs(1), abandoned_rx)
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
}

#[tokio::test]
async fn held_tool_keeps_lifecycle_open_until_the_cancelled_source_returns() {
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
    let sink = deadline_sink(&run, cancellation.clone(), Duration::from_secs(2));
    let workspace = run.workspace();
    let plan = RunPlan::new(vec![step("hold tool", Capabilities::default())]).unwrap();
    let active = driver.run_step(&sink, &plan, 0, &workspace);
    tokio::pin!(active);

    tokio::time::timeout(Duration::from_secs(2), async {
        tokio::select! {
            started = started_rx => started.unwrap(),
            result = &mut active => panic!("episode finished before tool start: {result:?}"),
        }
    })
    .await
    .expect("tool source starts");
    tokio::time::timeout(Duration::from_secs(3), async {
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
    let sink = deadline_sink(&run, cancellation.clone(), Duration::from_secs(2));
    let workspace = run.workspace();
    let plan = RunPlan::new(vec![step("hold tool", Capabilities::default())]).unwrap();
    let active = driver.run_step(&sink, &plan, 0, &workspace);
    tokio::pin!(active);

    tokio::time::timeout(Duration::from_secs(1), async {
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
    tokio::time::timeout(Duration::from_secs(1), async {
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
