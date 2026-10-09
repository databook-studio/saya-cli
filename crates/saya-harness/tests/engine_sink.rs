//! The engine's event sink — contract tests (M1-5, sink slice).
//!
//! Three guarantees, one per test:
//! 1. Usage accumulation keeps "not reported" distinct from zero: an
//!    `AgentEvent::Usage` figure no provider call reported stays `None` in
//!    the run totals — never `Some(0)` — while a reported zero stays a
//!    reported zero, and later reports sum in without folding unreported
//!    calls in as zeros.
//! 2. The wall-clock deadline trips a pause per tick: the run is `Executing`,
//!    the simulated clock passes the deadline, the next event emission pauses
//!    the run — journaled with `WallClockExceeded`, mirrored to the store —
//!    and a paused run never re-pauses.
//! 3. A store failure pauses rather than continuing: a mirror refused
//!    mid-flight fail-safe pauses the run, journals `StoreUnavailable`, holds
//!    a diagnostic instead of swallowing it, and leaves the machine's gates
//!    standing (a paused run cannot be completed past them).

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use saya_agent::{AgentEvent, AgentEventSink, TokenUsage, UsageCall};
use saya_harness::engine::{
    ElapsedClock, EngineEventSink, EngineSinkError, RunState, SinkBudgets, TransitionEvent,
    UsageTotals,
};
use saya_harness::fetch::DownloadBudget;
use saya_harness::journal::Journal;
use saya_store::{NewRun, RunBudgets, RunCapabilityFlags, RunStatus, RunStore, SqliteStateStore};
use saya_types::{PauseReason, RunEvent, RunId};
use tokio::sync::oneshot;

/// A per-test scratch root: the state database and the run directory both
/// live under it, and one cleanup covers both.
fn temp_root(label: &str) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("saya-engine-sink-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

/// A simulated wall clock. The engine owns the run's clock, so the sink
/// takes it as a closure; tests advance it in milliseconds instead of
/// sleeping a real deadline out.
#[derive(Clone)]
struct SimClock {
    base: Instant,
    elapsed_ms: Arc<AtomicU64>,
}

impl SimClock {
    fn new() -> Self {
        Self {
            base: Instant::now(),
            elapsed_ms: Arc::new(AtomicU64::new(0)),
        }
    }

    fn advance(&self, ms: u64) {
        self.elapsed_ms.fetch_add(ms, Ordering::SeqCst);
    }

    fn clock(&self) -> Instant {
        self.base + Duration::from_millis(self.elapsed_ms.load(Ordering::SeqCst))
    }
}

/// A working store whose run row already stands at `status` — the same
/// state the sink starts from. The store is the second guard over the same
/// machine, so the test walks its row through the same transitions.
async fn seeded_store(root: &Path, tag: &str, status: RunStatus) -> (Arc<SqliteStateStore>, RunId) {
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
    if status != RunStatus::Planned {
        store
            .set_run_status(&run_id, RunStatus::Approved, None)
            .await
            .unwrap();
    }
    if status == RunStatus::Executing {
        store
            .set_run_status(&run_id, RunStatus::Executing, None)
            .await
            .unwrap();
    }
    (store, run_id)
}

struct HeldDisplay {
    entered: std::sync::Mutex<Option<oneshot::Sender<()>>>,
    release: std::sync::Mutex<Option<oneshot::Receiver<()>>>,
}

#[async_trait::async_trait]
impl AgentEventSink for HeldDisplay {
    async fn emit(&self, _: AgentEvent) {
        let _ = self.entered.lock().unwrap().take().unwrap().send(());
        let release = self.release.lock().unwrap().take().unwrap();
        release.await.unwrap();
    }
}

/// The token ceiling pauses the run, and does it on the tick *after* the
/// tokens are spent.
///
/// An independent review found `--budget tokens.<endpoint>` parsed,
/// persisted to the store and the spec, and re-validated whenever a step
/// declared its own — and then never compared to anything at runtime. A user
/// who declared a ceiling got silence, on the one surface whose documented
/// meaning is that it pauses rather than silently stopping. This is the test
/// that would have caught it.
/// The usage a budget-paused run shows agrees with the ceiling that stopped
/// it. The sink journals each call's usage as it folds it, so the durable
/// record a reader aggregates (`saya run show` sums the journal's usage
/// events) and the totals the ceiling compared are the same numbers: the
/// journaled sum crosses the ceiling, equals the sink's own totals, and the
/// pause naming the budget is written after the usage that tripped it.
/// A run with no declared ceiling is not bounded into a pause by this check —
/// the guard must be inert when nothing was declared, or every unbudgeted run
/// would stop at zero.
/// One journaled usage event, shaped exactly as the sink journals a provider
/// call: the combined input-plus-output figure under the ceiling's
/// arithmetic, with every figure no call reported absent.
fn journaled_usage(tokens: u64) -> RunEvent {
    RunEvent::Usage {
        endpoint: "orchestrator".into(),
        tokens: Some(tokens),
        turns: None,
        tool_calls: None,
        cached_input_tokens: None,
        cache_creation_input_tokens: None,
    }
}

// --- the download budget's trip latch (S2) -----------------------------------

// --- the download spend's durable record --------------------------------------

/// One event to tick on — the sink's checks and its journaling ride every
/// emission, so a plain usage event moves both.
fn an_event() -> AgentEvent {
    AgentEvent::Usage {
        call: UsageCall::Answer,
        usage: TokenUsage::new(1, 1),
    }
}

/// The levels a journal holds, in write order.
fn journaled_levels(journal: &Journal) -> Vec<u64> {
    journal
        .read()
        .unwrap()
        .iter()
        .filter_map(|event| match event {
            RunEvent::DownloadedBytes { bytes } => Some(*bytes),
            _ => None,
        })
        .collect()
}

/// The sink journals the wallet's consumed level as it grows — the durable
/// record a resume seeds the wallet from, the role the usage record plays
/// for the token ceiling. One event per level the sink observes: an emit
/// with no growth records nothing, and the event carries the level the
/// wallet stood at, never a per-tick delta.
/// A resumed sink never re-journals the level its record already holds.
/// The wallet `resume` seeds — 60 of its 100 already spent, journaled by
/// the invocation that spent it — is the baseline the sink journals growth
/// past, so the carried spend stands in the record once. Re-journaling it
/// would double it in the next resume's carry.
/// The record holds the spend before the pause that stopped it: the
/// tripping refusal's tick journals the wallet's level first, then pauses.
/// A reader of the record sees the bytes the run claimed, then the budget
/// that stopped it — the shape a resume's carry reads.
#[path = "engine_sink/clock.rs"]
mod clock;

#[path = "engine_sink/failed_attempt_usage.rs"]
mod failed_attempt_usage;

#[path = "engine_sink/budget_latch.rs"]
mod budget_latch;
#[path = "engine_sink/download_budget.rs"]
mod download_budget;
#[path = "engine_sink/journal.rs"]
mod journal;
#[path = "engine_sink/lifecycle_clock.rs"]
mod lifecycle_clock;
#[path = "engine_sink/store_failure.rs"]
mod store_failure;
#[path = "engine_sink/token_ceiling.rs"]
mod token_ceiling;
#[path = "engine_sink/usage.rs"]
mod usage;
#[path = "engine_sink/wall_clock.rs"]
mod wall_clock;
