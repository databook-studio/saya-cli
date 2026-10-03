use super::*;
use saya_agent::{
    BudgetDimension, CancellationReason, ChatMessage, ChatRequest, ChatResponse, ProviderStream,
    TokenUsage,
};
use saya_harness::engine::ElapsedClock;
use saya_store::{
    NewRun, RunRecord, RunStatus, RunStepRecord, RunStepStatus, RunStore, RunSummary, RunUsage,
    StoreError,
};
use saya_types::RunId;
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
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

struct ReceiptThenTool {
    journal_path: PathBuf,
    kept_journal: PathBuf,
    calls: AtomicUsize,
}

#[async_trait]
impl ChatProvider for ReceiptThenTool {
    fn name(&self) -> &str {
        "receipt-then-tool"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let first = self.calls.fetch_add(1, Ordering::SeqCst) == 0;
        if first {
            fs::copy(&self.journal_path, &self.kept_journal).unwrap();
            fs::remove_file(&self.journal_path).unwrap();
            fs::create_dir(&self.journal_path).unwrap();
        } else {
            return Ok(ChatResponse::new(ChatMessage::text("assistant", "done")));
        }
        let mut response = ChatResponse::new(ChatMessage {
            role: "assistant".into(),
            content: String::new(),
            tool_calls: vec![call("probe")],
            tool_call_id: None,
        });
        response.usage = Some(TokenUsage::new(1, 0));
        Ok(response)
    }
}

#[tokio::test]
async fn accounting_append_failure_stops_before_the_next_tool_and_keeps_journal_error() {
    let run = approved_run("accounting-journal-failure").await;
    let journal_path = run.run_dir.join("events.ndjson");
    let kept_journal = run.run_dir.join("events-before-failure.ndjson");
    let provider = ReceiptThenTool {
        journal_path,
        kept_journal: kept_journal.clone(),
        calls: AtomicUsize::new(0),
    };
    let executor = Arc::new(RecordingTools::default());
    let tools = vec![toolset(
        executor.clone(),
        vec![def("probe", LocalStateEffect::None)],
    )];
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
            store: run.store.clone(),
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
        run.store.clone(),
        SinkBudgets {
            wall_clock: None,
            token_ceiling: None,
            download_budget: None,
            carried_usage: UsageTotals::default(),
        },
        std::time::Instant::now,
    )
    .with_cancellation(cancellation.clone());
    let plan = RunPlan::new(vec![step(
        "account usage before tool",
        Capabilities::default(),
    )])
    .unwrap();

    let error = driver
        .run_step(&sink, &plan, 0, &run.workspace())
        .await
        .unwrap_err();

    assert_eq!(
        cancellation.reason(),
        Some(CancellationReason::OwnerFailure),
        "a missing usage receipt stops the same agent/tool owner before another effect"
    );
    assert!(
        executor.calls().is_empty(),
        "the tool must not run after receipt I/O failed"
    );
    assert!(matches!(
        &error,
        EpisodeError::OwnerFailureDiagnostic { diagnostic: error, .. }
            if matches!(error.as_ref(), saya_harness::engine::EngineSinkError::Journal { .. })
    ));
    let retained = fs::read_to_string(kept_journal).unwrap();
    let retained_events = retained
        .lines()
        .map(|line| serde_json::from_str::<RunEvent>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        retained_events,
        vec![RunEvent::StepStarted { step: 0 }],
        "the pre-failure journal remains intact and no successful usage receipt is fabricated"
    );
    assert!(
        fs::metadata(run.run_dir.join("events.ndjson"))
            .unwrap()
            .is_dir()
    );
    let _ = fs::remove_dir_all(run.root);
}

#[tokio::test]
async fn approved_clock_observation_failure_preserves_diagnostic_without_source_work() {
    let run = approved_run("approved-clock-write-failure").await;
    fs::create_dir(run.run_dir.join("events.ndjson")).unwrap();
    let provider = ScriptProvider::new(Vec::new());
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
            store: run.store.clone(),
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
        run.store.clone(),
        SinkBudgets {
            wall_clock: None,
            token_ceiling: None,
            download_budget: None,
            carried_usage: UsageTotals::default(),
        },
        std::time::Instant::now,
    )
    .with_elapsed_clock(ElapsedClock::arm_now(Duration::from_secs(30)).unwrap())
    .with_cancellation(cancellation.clone());
    let plan = RunPlan::new(vec![step(
        "clock must persist before work",
        Capabilities::default(),
    )])
    .unwrap();

    let error = driver
        .run_step(&sink, &plan, 0, &run.workspace())
        .await
        .unwrap_err();

    assert!(matches!(error, EpisodeError::SinkDiagnostic { .. }));
    assert!(provider.requests().is_empty(), "the source is not polled");
    assert_eq!(sink.state(), RunState::Approved);
    assert!(matches!(
        sink.take_diagnostic().as_deref(),
        Some(saya_harness::engine::EngineSinkError::Journal { .. })
    ));
    let _ = fs::remove_dir_all(run.root);
}

struct HoldFinalFailedMirror {
    inner: Arc<SqliteStateStore>,
    failures: AtomicUsize,
    entered: std::sync::Mutex<Option<oneshot::Sender<()>>>,
    release: std::sync::Mutex<Option<oneshot::Receiver<()>>>,
}

#[async_trait]
impl RunStore for HoldFinalFailedMirror {
    async fn create_run(&self, run: NewRun) -> Result<RunRecord, StoreError> {
        RunStore::create_run(self.inner.as_ref(), run).await
    }

    async fn get_run(&self, id: &RunId) -> Result<Option<RunRecord>, StoreError> {
        RunStore::get_run(self.inner.as_ref(), id).await
    }

    async fn list_runs(&self) -> Result<Vec<RunSummary>, StoreError> {
        RunStore::list_runs(self.inner.as_ref()).await
    }

    async fn set_run_status(
        &self,
        id: &RunId,
        status: RunStatus,
        failure_code: Option<saya_types::RunFailureCode>,
    ) -> Result<(), StoreError> {
        RunStore::set_run_status(self.inner.as_ref(), id, status, failure_code).await
    }

    async fn set_run_usage(&self, id: &RunId, usage: RunUsage) -> Result<(), StoreError> {
        RunStore::set_run_usage(self.inner.as_ref(), id, usage).await
    }

    async fn upsert_step(
        &self,
        id: &RunId,
        step: usize,
        status: RunStepStatus,
    ) -> Result<(), StoreError> {
        if status == RunStepStatus::Failed
            && self.failures.fetch_add(1, Ordering::SeqCst) + 1
                == saya_harness::engine::MAX_EPISODE_ATTEMPTS
        {
            let _ = self.entered.lock().unwrap().take().unwrap().send(());
            let release = self.release.lock().unwrap().take().unwrap();
            release.await.unwrap();
        }
        RunStore::upsert_step(self.inner.as_ref(), id, step, status).await
    }

    async fn set_step_usage(
        &self,
        id: &RunId,
        step: usize,
        usage: RunUsage,
    ) -> Result<(), StoreError> {
        RunStore::set_step_usage(self.inner.as_ref(), id, step, usage).await
    }

    async fn list_steps(&self, id: &RunId) -> Result<Vec<RunStepRecord>, StoreError> {
        RunStore::list_steps(self.inner.as_ref(), id).await
    }
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
