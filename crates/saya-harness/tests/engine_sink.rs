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
    EngineEventSink, EngineSinkError, RunState, TransitionEvent, UsageTotals,
};
use saya_harness::journal::Journal;
use saya_store::{NewRun, RunBudgets, RunCapabilityFlags, RunStatus, RunStore, SqliteStateStore};
use saya_types::{PauseReason, RunEvent, RunId};

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

#[tokio::test]
async fn usage_accumulation_keeps_not_reported_distinct_from_zero() {
    let clock = SimClock::new();
    let root = temp_root("usage");
    let (store, run_id) = seeded_store(&root, "usage", RunStatus::Executing).await;
    fs::create_dir_all(root.join("run")).unwrap();
    let clock_for_sink = clock.clone();
    let sink = EngineEventSink::new(
        run_id,
        RunState::Executing,
        Journal::open(root.join("run")),
        store,
        None,
        move || clock_for_sink.clock(),
    );

    // Before any provider call: nothing is known, and nothing is read as
    // zero — the optional figures are `None`, not `Some(0)`.
    assert_eq!(sink.usage(), UsageTotals::default());

    AgentEventSink::emit(
        &sink,
        AgentEvent::Usage {
            call: UsageCall::Answer,
            usage: TokenUsage::new(100, 30),
        },
    )
    .await;
    let totals = sink.usage();
    assert_eq!(totals.input_tokens, 100);
    assert_eq!(totals.output_tokens, 30);
    assert_eq!(
        totals.cached_input_tokens, None,
        "an unreported figure must stay unknown, never zero"
    );

    // A *reported* zero is a number, not unknown: it survives as `Some(0)`.
    AgentEventSink::emit(
        &sink,
        AgentEvent::Usage {
            call: UsageCall::Extraction,
            usage: TokenUsage::new(10, 2).with_cached_input(Some(0)),
        },
    )
    .await;
    assert_eq!(
        sink.usage().cached_input_tokens,
        Some(0),
        "a reported zero is a number, not 'not reported'"
    );

    // Later reports sum in; a call that reports nothing never folds in as a
    // zero for that figure.
    AgentEventSink::emit(
        &sink,
        AgentEvent::Usage {
            call: UsageCall::Answer,
            usage: TokenUsage::new(5, 1)
                .with_cached_input(Some(5))
                .with_reasoning(Some(3)),
        },
    )
    .await;
    AgentEventSink::emit(
        &sink,
        AgentEvent::Usage {
            call: UsageCall::Answer,
            usage: TokenUsage::new(7, 0).with_cache_creation(Some(2)),
        },
    )
    .await;
    let totals = sink.usage();
    assert_eq!(totals.input_tokens, 122);
    assert_eq!(totals.output_tokens, 33);
    assert_eq!(
        totals.cached_input_tokens,
        Some(5),
        "the last call did not report cache reads; they must not fold in as zero"
    );
    assert_eq!(totals.cache_creation_input_tokens, Some(2));
    assert_eq!(totals.reasoning_tokens, Some(3));

    // Usage counting never moves the machine: the run still executes.
    assert_eq!(sink.state(), RunState::Executing);

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn the_wall_clock_deadline_trips_a_pause_per_tick() {
    let clock = SimClock::new();
    let root = temp_root("deadline");
    let (store, run_id) = seeded_store(&root, "deadline", RunStatus::Approved).await;
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).unwrap();
    let clock_for_sink = clock.clone();
    let sink = EngineEventSink::new(
        run_id.clone(),
        RunState::Approved,
        Journal::open(&run_dir),
        store.clone(),
        Some(Duration::from_millis(100)),
        move || clock_for_sink.clock(),
    );

    // Executing, before the deadline: a tick is a no-op — no pause, no
    // journal line.
    sink.record(TransitionEvent::Begin).await.unwrap();
    assert_eq!(sink.state(), RunState::Executing);
    AgentEventSink::emit(
        &sink,
        AgentEvent::Usage {
            call: UsageCall::Answer,
            usage: TokenUsage::new(1, 1),
        },
    )
    .await;
    assert_eq!(sink.state(), RunState::Executing);
    assert_eq!(
        Journal::open(&run_dir).read().unwrap(),
        Vec::<RunEvent>::new(),
        "a tick before the deadline must not pause"
    );

    // Past the deadline, the very next tick pauses the run — once, with the
    // wall-clock reason, in the journal and in the store.
    clock.advance(150);
    AgentEventSink::emit(
        &sink,
        AgentEvent::Usage {
            call: UsageCall::Answer,
            usage: TokenUsage::new(1, 1),
        },
    )
    .await;
    assert_eq!(sink.state(), RunState::Paused);
    assert_eq!(
        Journal::open(&run_dir).read().unwrap(),
        vec![RunEvent::Paused {
            reason: PauseReason::WallClockExceeded
        }],
        "the deadline pause must be journaled exactly once, with its reason"
    );
    let record = store.get_run(&run_id).await.unwrap().unwrap();
    assert_eq!(
        record.status,
        RunStatus::Paused,
        "the pause must be mirrored to the store"
    );

    // A paused run does not re-pause: further ticks write nothing new.
    clock.advance(10_000);
    AgentEventSink::emit(
        &sink,
        AgentEvent::Usage {
            call: UsageCall::Answer,
            usage: TokenUsage::new(1, 1),
        },
    )
    .await;
    assert_eq!(Journal::open(&run_dir).read().unwrap().len(), 1);

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn a_store_failure_pauses_rather_than_continuing() {
    // A store that can never open — its parent path is a regular file, the
    // `store_open_failed` recipe — so every mirror fails fast, permanently.
    let root = temp_root("store-failure");
    fs::write(root.join("blocker"), b"x").unwrap();
    let failing: Arc<dyn RunStore> =
        Arc::new(SqliteStateStore::new(root.join("blocker/state.sqlite3")));
    let run_id = RunId::parse("run-store-failure").unwrap();
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).unwrap();
    let sink = EngineEventSink::new(
        run_id,
        RunState::Approved,
        Journal::open(&run_dir),
        failing,
        None,
        Instant::now,
    );

    // The first mirror fails while the run is mid-flight (approved →
    // executing). The sink must pause with a diagnostic — never continue
    // executing with the store refusing writes.
    let error = sink.record(TransitionEvent::Begin).await.unwrap_err();
    assert!(
        matches!(error, EngineSinkError::Store { .. }),
        "expected a store error, got {error:?}"
    );
    assert_eq!(
        sink.state(),
        RunState::Paused,
        "the run must pause on a store failure"
    );
    assert_eq!(
        Journal::open(&run_dir).read().unwrap(),
        vec![RunEvent::Paused {
            reason: PauseReason::StoreUnavailable
        }],
        "the store-unavailable pause must be journaled, with its reason"
    );
    assert!(
        matches!(
            sink.take_diagnostic().as_deref(),
            Some(EngineSinkError::Store { .. })
        ),
        "the store failure must be held as a diagnostic, not swallowed"
    );

    // The gate holds: from paused, completing is refused by the machine, and
    // the refusal writes nothing to the journal.
    let refused = sink.record(TransitionEvent::Complete).await.unwrap_err();
    assert!(
        matches!(refused, EngineSinkError::Transition { .. }),
        "expected a transition refusal from paused, got {refused:?}"
    );
    assert_eq!(Journal::open(&run_dir).read().unwrap().len(), 1);

    let _ = fs::remove_dir_all(root);
}
