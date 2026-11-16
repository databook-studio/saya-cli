use std::{
    fs,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use saya_harness::engine::{
    EngineEventSink, EngineSinkError, RunState, SinkBudgets, TransitionEvent, UsageTotals,
};
use saya_harness::journal::Journal;
use saya_store::{NewRun, RunBudgets, RunCapabilityFlags, RunStatus, RunStore, SqliteStateStore};
use saya_types::{RunEvent, RunId};

fn temp_root(label: &str) -> std::path::PathBuf {
    let root =
        std::env::temp_dir().join(format!("saya-engine-clock-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

async fn seeded_store(root: &Path, tag: &str) -> (Arc<SqliteStateStore>, RunId) {
    let store = Arc::new(SqliteStateStore::new(root.join("state.sqlite3")));
    let run_id = RunId::parse(&format!("run-{tag}")).unwrap();
    store
        .create_run(NewRun {
            id: run_id.clone(),
            capabilities: RunCapabilityFlags::default(),
            budgets: RunBudgets::default(),
        })
        .await
        .unwrap();
    (store, run_id)
}

#[tokio::test]
async fn approval_journals_the_clock_origin_before_execution_begins() {
    let root = temp_root("clock-origin");
    let (store, run_id) = seeded_store(&root, "clock-origin").await;
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).unwrap();
    let journal = Journal::open(&run_dir);
    journal.append(&RunEvent::RunStarted).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let elapsed =
        saya_harness::engine::ElapsedClock::arm(Duration::from_secs(30), now, Instant::now())
            .unwrap();
    let sink = EngineEventSink::new(
        run_id,
        RunState::Planned,
        journal.clone(),
        store,
        SinkBudgets {
            wall_clock: Some(Duration::from_secs(30)),
            token_ceiling: None,
            download_budget: None,
            carried_usage: UsageTotals::default(),
        },
        Instant::now,
    )
    .with_elapsed_clock(elapsed);

    sink.record(TransitionEvent::Approve { scopes: vec![] })
        .await
        .unwrap();

    let events = journal.read().unwrap();
    assert!(matches!(events.as_slice(), [
        RunEvent::RunStarted,
        RunEvent::PlanApproved { scopes: Some(scopes) },
        RunEvent::WallClockObserved { .. },
    ] if scopes.is_empty()));
    assert_eq!(sink.state(), RunState::Approved);
    let _ = fs::remove_dir_all(root);
}

struct RefuseRunStatusStore(SqliteStateStore);

#[async_trait::async_trait]
impl RunStore for RefuseRunStatusStore {
    async fn create_run(
        &self,
        run: NewRun,
    ) -> Result<saya_store::RunRecord, saya_store::StoreError> {
        self.0.create_run(run).await
    }

    async fn get_run(
        &self,
        id: &RunId,
    ) -> Result<Option<saya_store::RunRecord>, saya_store::StoreError> {
        self.0.get_run(id).await
    }

    async fn list_runs(&self) -> Result<Vec<saya_store::RunSummary>, saya_store::StoreError> {
        self.0.list_runs().await
    }

    async fn set_run_status(
        &self,
        _: &RunId,
        _: RunStatus,
        _: Option<saya_types::RunFailureCode>,
    ) -> Result<(), saya_store::StoreError> {
        Err(saya_store::StoreError::unavailable())
    }

    async fn set_run_usage(
        &self,
        id: &RunId,
        usage: saya_store::RunUsage,
    ) -> Result<(), saya_store::StoreError> {
        self.0.set_run_usage(id, usage).await
    }

    async fn upsert_step(
        &self,
        id: &RunId,
        step: usize,
        status: saya_store::RunStepStatus,
    ) -> Result<(), saya_store::StoreError> {
        self.0.upsert_step(id, step, status).await
    }

    async fn set_step_usage(
        &self,
        id: &RunId,
        step: usize,
        usage: saya_store::RunUsage,
    ) -> Result<(), saya_store::StoreError> {
        self.0.set_step_usage(id, step, usage).await
    }

    async fn list_steps(
        &self,
        id: &RunId,
    ) -> Result<Vec<saya_store::RunStepRecord>, saya_store::StoreError> {
        self.0.list_steps(id).await
    }
}

#[tokio::test]
async fn journaled_clock_origin_survives_an_approval_mirror_failure() {
    let root = temp_root("clock-mirror-fault");
    let (store, run_id) = seeded_store(&root, "clock-mirror-fault").await;
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).unwrap();
    let journal = Journal::open(&run_dir);
    journal.append(&RunEvent::RunStarted).unwrap();
    let failing_store: Arc<dyn RunStore> = Arc::new(RefuseRunStatusStore((*store).clone()));
    let ceiling = Duration::from_secs(30);
    let origin = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let elapsed = saya_harness::engine::ElapsedClock::arm(ceiling, origin, Instant::now()).unwrap();
    let sink = EngineEventSink::new(
        run_id,
        RunState::Planned,
        journal.clone(),
        failing_store,
        SinkBudgets {
            wall_clock: Some(ceiling),
            token_ceiling: None,
            download_budget: None,
            carried_usage: UsageTotals::default(),
        },
        Instant::now,
    )
    .with_elapsed_clock(elapsed);

    assert!(matches!(
        sink.record(TransitionEvent::Approve { scopes: vec![] })
            .await,
        Err(EngineSinkError::Store { .. })
    ));
    let events = journal.read().unwrap();
    assert!(matches!(events.as_slice(), [
        RunEvent::RunStarted,
        RunEvent::PlanApproved { scopes: Some(scopes) },
        RunEvent::WallClockObserved { origin_unix_ms, .. },
    ] if scopes.is_empty() && *origin_unix_ms == origin));
    assert_eq!(sink.state(), RunState::Approved);
    let remaining =
        saya_harness::engine::ElapsedClock::resume(&events, ceiling, origin, Instant::now())
            .unwrap();
    assert_eq!(remaining.remaining(), ceiling);
    let _ = fs::remove_dir_all(root);
}
