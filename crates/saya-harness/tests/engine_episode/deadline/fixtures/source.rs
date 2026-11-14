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

/// How long an event that follows another within milliseconds may take before
/// a test calls it a hang. The slowest of these gaps, the episode's setup
/// before its source starts, took half a second on a host running several
/// build paths; only a hang takes ten.
const HANG_BOUND: Duration = Duration::from_secs(10);

fn is_elapsed_time_stop(result: &Result<(), EpisodeError>) -> bool {
    matches!(
        result,
        Err(EpisodeError::BudgetStopped {
            step: 0,
            reason: PauseReason::WallClockExceeded
        })
    )
}

/// A deadline test arms its ceiling just before the episode starts, and the
/// episode's own setup, its journal and store writes, runs inside that
/// ceiling. On a loaded host the setup alone can outlast a short ceiling,
/// which says nothing about the property under test. An attempt returns `false`
/// when its ceiling expired before its source started, and the test runs it
/// again with the next, longer ceiling. Every assertion in the attempt whose
/// source did start holds at full strength.
async fn with_ceiling_beyond_setup<F, Fut>(ceilings: &[Duration], attempt: F)
where
    F: Fn(Duration) -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for ceiling in ceilings {
        if attempt(*ceiling).await {
            return;
        }
    }
    panic!("the episode did not start its source within any of {ceilings:?}");
}
