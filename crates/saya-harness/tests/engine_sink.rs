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
    EngineEventSink, EngineSinkError, RunState, SinkBudgets, TransitionEvent, UsageTotals,
};
use saya_harness::fetch::DownloadBudget;
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
        SinkBudgets {
            wall_clock: None,
            token_ceiling: None,
            download_budget: None,
            carried_usage: UsageTotals::default(),
        },
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
        SinkBudgets {
            wall_clock: Some(Duration::from_millis(100)),
            token_ceiling: None,
            download_budget: None,
            carried_usage: UsageTotals::default(),
        },
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
    let events = Journal::open(&run_dir).read().unwrap();
    assert!(
        matches!(events.as_slice(), [RunEvent::Usage { .. }]),
        "a tick before the deadline must not pause; the call's usage is \
         journaled as the durable record, nothing more: {events:?}"
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
    let events = Journal::open(&run_dir).read().unwrap();
    assert_eq!(
        events,
        vec![
            RunEvent::Usage {
                endpoint: "orchestrator".into(),
                tokens: Some(2),
                turns: None,
                tool_calls: None,
                cached_input_tokens: None,
                cache_creation_input_tokens: None,
            },
            RunEvent::Usage {
                endpoint: "orchestrator".into(),
                tokens: Some(2),
                turns: None,
                tool_calls: None,
                cached_input_tokens: None,
                cache_creation_input_tokens: None,
            },
            RunEvent::Paused {
                reason: PauseReason::WallClockExceeded
            },
        ],
        "each call's usage is journaled, and the deadline pause exactly once, with its reason"
    );
    let record = store.get_run(&run_id).await.unwrap().unwrap();
    assert_eq!(
        record.status,
        RunStatus::Paused,
        "the pause must be mirrored to the store"
    );

    // A paused run does not re-pause: further ticks never write a second
    // pause — the only thing a later usage event adds is its own record.
    clock.advance(10_000);
    AgentEventSink::emit(
        &sink,
        AgentEvent::Usage {
            call: UsageCall::Answer,
            usage: TokenUsage::new(1, 1),
        },
    )
    .await;
    let events = Journal::open(&run_dir).read().unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, RunEvent::Paused { .. }))
            .count(),
        1,
        "a paused run never re-pauses: {events:?}"
    );

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
        SinkBudgets {
            wall_clock: None,
            token_ceiling: None,
            download_budget: None,
            carried_usage: UsageTotals::default(),
        },
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

/// The token ceiling pauses the run, and does it on the tick *after* the
/// tokens are spent.
///
/// An independent review found `--budget tokens.<endpoint>` parsed,
/// persisted to the store and the spec, and re-validated whenever a step
/// declared its own — and then never compared to anything at runtime. A user
/// who declared a ceiling got silence, on the one surface whose documented
/// meaning is that it pauses rather than silently stopping. This is the test
/// that would have caught it.
#[tokio::test]
async fn the_token_ceiling_pauses_the_run_once_it_is_spent() {
    let root = temp_root("tokens");
    let (store, run_id) = seeded_store(&root, "tokens", RunStatus::Approved).await;
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).unwrap();
    let sink = EngineEventSink::new(
        run_id.clone(),
        RunState::Approved,
        Journal::open(&run_dir),
        store.clone(),
        SinkBudgets {
            wall_clock: None,
            token_ceiling: Some(150),
            download_budget: None,
            carried_usage: UsageTotals::default(),
        },
        Instant::now,
    );
    sink.record(TransitionEvent::Begin).await.unwrap();
    assert_eq!(sink.state(), RunState::Executing);

    // Under the ceiling: counted, and the run keeps executing.
    AgentEventSink::emit(
        &sink,
        AgentEvent::Usage {
            call: UsageCall::Answer,
            usage: TokenUsage::new(60, 40),
        },
    )
    .await;
    assert_eq!(sink.state(), RunState::Executing, "100 of 150 is not spent");

    // Crossing it: input and output are summed, because the budget is what
    // the run costs and a ceiling on half of it bounds nothing in particular.
    AgentEventSink::emit(
        &sink,
        AgentEvent::Usage {
            call: UsageCall::Answer,
            usage: TokenUsage::new(40, 20),
        },
    )
    .await;
    assert_eq!(
        sink.state(),
        RunState::Paused,
        "160 tokens against a 150 ceiling must pause the run"
    );

    // The journal says *why*, so `run show` and a resume can both tell this
    // pause apart from a wall-clock one.
    let events = Journal::open(&run_dir).read().unwrap();
    assert!(
        events.iter().any(|event| matches!(
            event,
            RunEvent::Paused {
                reason: PauseReason::BudgetExhausted,
                ..
            }
        )),
        "the pause must name the budget: {events:?}"
    );

    let _ = fs::remove_dir_all(root);
}

/// The usage a budget-paused run shows agrees with the ceiling that stopped
/// it. The sink journals each call's usage as it folds it, so the durable
/// record a reader aggregates (`saya run show` sums the journal's usage
/// events) and the totals the ceiling compared are the same numbers: the
/// journaled sum crosses the ceiling, equals the sink's own totals, and the
/// pause naming the budget is written after the usage that tripped it.
#[tokio::test]
async fn the_usage_a_budget_pause_shows_agrees_with_the_ceiling_that_stopped_it() {
    let root = temp_root("tokens-display");
    let (store, run_id) = seeded_store(&root, "tokens-display", RunStatus::Approved).await;
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).unwrap();
    let sink = EngineEventSink::new(
        run_id,
        RunState::Approved,
        Journal::open(&run_dir),
        store,
        SinkBudgets {
            wall_clock: None,
            token_ceiling: Some(150),
            download_budget: None,
            carried_usage: UsageTotals::default(),
        },
        Instant::now,
    );
    sink.record(TransitionEvent::Begin).await.unwrap();

    // 100 of 150, then the crossing call — the same shape the pause test
    // above drives, so both readings of the same run must agree.
    AgentEventSink::emit(
        &sink,
        AgentEvent::Usage {
            call: UsageCall::Answer,
            usage: TokenUsage::new(60, 40),
        },
    )
    .await;
    AgentEventSink::emit(
        &sink,
        AgentEvent::Usage {
            call: UsageCall::Answer,
            usage: TokenUsage::new(40, 20),
        },
    )
    .await;
    assert_eq!(sink.state(), RunState::Paused);

    let events = Journal::open(&run_dir).read().unwrap();
    let journaled: u64 = events
        .iter()
        .map(|event| match event {
            RunEvent::Usage {
                tokens: Some(tokens),
                ..
            } => *tokens,
            _ => 0,
        })
        .sum();
    assert_eq!(
        journaled, 160,
        "the journal must record what the run spent: {events:?}"
    );
    assert!(
        journaled >= 150,
        "usage shown for a budget pause must agree with the ceiling that stopped it"
    );
    let totals = sink.usage();
    assert_eq!(
        totals.input_tokens.saturating_add(totals.output_tokens),
        journaled,
        "the ceiling's arithmetic and the durable record must agree"
    );
    for event in &events {
        if let RunEvent::Usage { endpoint, .. } = event {
            assert_eq!(
                endpoint, "orchestrator",
                "every episode calls the one endpoint the ceiling sums: {events:?}"
            );
        }
    }
    let pause_index = events
        .iter()
        .position(|event| {
            matches!(
                event,
                RunEvent::Paused {
                    reason: PauseReason::BudgetExhausted
                }
            )
        })
        .expect("the pause must be journaled");
    let usage_index = events
        .iter()
        .position(|event| matches!(event, RunEvent::Usage { .. }))
        .expect("the usage must be journaled");
    assert!(
        usage_index < pause_index,
        "the usage that tripped the ceiling precedes the pause naming it: {events:?}"
    );

    let _ = fs::remove_dir_all(root);
}

/// A run with no declared ceiling is not bounded into a pause by this check —
/// the guard must be inert when nothing was declared, or every unbudgeted run
/// would stop at zero.
#[tokio::test]
async fn no_declared_ceiling_means_no_token_pause() {
    let root = temp_root("notokens");
    let (store, run_id) = seeded_store(&root, "notokens", RunStatus::Approved).await;
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).unwrap();
    let sink = EngineEventSink::new(
        run_id,
        RunState::Approved,
        Journal::open(&run_dir),
        store,
        SinkBudgets {
            wall_clock: None,
            token_ceiling: None,
            download_budget: None,
            carried_usage: UsageTotals::default(),
        },
        Instant::now,
    );
    sink.record(TransitionEvent::Begin).await.unwrap();
    AgentEventSink::emit(
        &sink,
        AgentEvent::Usage {
            call: UsageCall::Answer,
            usage: TokenUsage::new(10_000_000, 10_000_000),
        },
    )
    .await;
    assert_eq!(sink.state(), RunState::Executing);
    let _ = fs::remove_dir_all(root);
}

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

/// The seeding path's twin of the accumulation test above: a sink seeded
/// from its journal — what a resume does — keeps "not reported" distinct
/// from zero across the seam. The journal carries only the combined
/// input-plus-output figure per call, so the seeded totals hold that spend
/// whole (`carried_tokens`), the optional figures no recorded call reported
/// stay `None`, no split is fabricated, and the invocation's own reports
/// fold in under the same rule afterwards.
#[tokio::test]
async fn seeding_from_the_journal_keeps_not_reported_distinct_from_zero() {
    let root = temp_root("seeded-usage");
    let (store, run_id) = seeded_store(&root, "seeded-usage", RunStatus::Executing).await;
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).unwrap();
    let journaled = vec![journaled_usage(100), journaled_usage(60)];
    let journal = Journal::open(&run_dir);
    for event in &journaled {
        journal.append(event).unwrap();
    }
    let sink = EngineEventSink::new(
        run_id,
        RunState::Executing,
        Journal::open(&run_dir),
        store,
        SinkBudgets {
            wall_clock: None,
            token_ceiling: None,
            download_budget: None,
            carried_usage: UsageTotals::from_journal(&journaled),
        },
        Instant::now,
    );

    // The seeded totals: the record's combined spend, every optional figure
    // still unknown — never zero — and no input/output split the journal
    // never held.
    let seeded = sink.usage();
    assert_eq!(seeded.carried_tokens, 160);
    assert_eq!(
        (seeded.input_tokens, seeded.output_tokens),
        (0, 0),
        "the record holds no split, and none is fabricated"
    );
    assert_eq!(
        seeded.cached_input_tokens, None,
        "a figure no recorded call reported stays unknown, never zero"
    );
    assert_eq!(seeded.cache_creation_input_tokens, None);
    assert_eq!(seeded.reasoning_tokens, None, "the journal carries none");

    // A *reported* zero from this invocation is a number, not unknown: it
    // survives as `Some(0)` on the seeded sink too.
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

    // Later live reports sum in under the same rule; a call that reports
    // nothing never folds in as a zero, and the carried spend stands apart
    // from the invocation's own figures.
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
    let totals = sink.usage();
    assert_eq!(totals.carried_tokens, 160, "seeding is not re-folded");
    assert_eq!(totals.input_tokens, 15);
    assert_eq!(totals.output_tokens, 3);
    assert_eq!(
        totals.cached_input_tokens,
        Some(5),
        "the last call did not report cache reads; they must not fold in as zero"
    );
    assert_eq!(totals.reasoning_tokens, Some(3));

    let _ = fs::remove_dir_all(root);
}

/// The ceiling binds the run's whole spend, not each invocation's: a sink
/// seeded from its journal — the seeding a resume does — pauses on the next
/// tick even though the spend this invocation has made is far under the
/// ceiling. Without the seed, a resumed invocation re-armed the ceiling in
/// full and could spend it again; this is the sink-side form of the defect
/// the resume tests drive end to end.
#[tokio::test]
async fn the_ceiling_binds_the_run_s_whole_spend_not_each_invocation_s() {
    let root = temp_root("seeded-ceiling");
    let (store, run_id) = seeded_store(&root, "seeded-ceiling", RunStatus::Executing).await;
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).unwrap();
    // The journal a ceiling-paused run leaves: each call's usage, then the
    // pause — 160 tokens spent against the 150 ceiling.
    let journaled = vec![journaled_usage(100), journaled_usage(60)];
    let journal = Journal::open(&run_dir);
    for event in &journaled {
        journal.append(event).unwrap();
    }
    let sink = EngineEventSink::new(
        run_id,
        RunState::Executing,
        journal,
        store,
        SinkBudgets {
            wall_clock: None,
            token_ceiling: Some(150),
            download_budget: None,
            carried_usage: UsageTotals::from_journal(&journaled),
        },
        Instant::now,
    );
    assert_eq!(
        sink.usage().carried_tokens,
        160,
        "the journal seeds the spend the run already made"
    );

    // One token this invocation — far under a fresh 150 — and the run
    // pauses: the ceiling compares the run's 161, not the invocation's 1.
    AgentEventSink::emit(
        &sink,
        AgentEvent::Usage {
            call: UsageCall::Answer,
            usage: TokenUsage::new(1, 0),
        },
    )
    .await;
    assert_eq!(
        sink.state(),
        RunState::Paused,
        "161 tokens against a 150 ceiling must pause the run, though the \
         invocation itself spent one"
    );
    assert!(matches!(
        Journal::open(&run_dir).read().unwrap().last(),
        Some(RunEvent::Paused {
            reason: PauseReason::BudgetExhausted
        })
    ));

    let _ = fs::remove_dir_all(root);
}

// --- the download budget's trip latch (S2) -----------------------------------

/// The trip latch pauses the run on the first tick after the trip, with the
/// same reason the token ceiling uses. The sink holds a *clone* of the
/// wallet the fetch-capable executors hold — clone-shares-state — so the
/// test trips it through a second clone exactly the way a refused download
/// inside a step would.
#[tokio::test]
async fn the_download_budget_s_trip_latch_pauses_the_run_once_it_trips() {
    let root = temp_root("download-latch");
    let (store, run_id) = seeded_store(&root, "download-latch", RunStatus::Approved).await;
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).unwrap();
    let budget = DownloadBudget::new(100);
    let sink = EngineEventSink::new(
        run_id.clone(),
        RunState::Approved,
        Journal::open(&run_dir),
        store.clone(),
        SinkBudgets {
            wall_clock: None,
            token_ceiling: None,
            download_budget: Some(budget.clone()),
            carried_usage: UsageTotals::default(),
        },
        Instant::now,
    );
    sink.record(TransitionEvent::Begin).await.unwrap();
    assert_eq!(sink.state(), RunState::Executing);

    // A run consuming under its budget with nothing refused keeps executing.
    assert!(budget.claim(60));
    AgentEventSink::emit(
        &sink,
        AgentEvent::Usage {
            call: UsageCall::Answer,
            usage: TokenUsage::new(1, 1),
        },
    )
    .await;
    assert_eq!(sink.state(), RunState::Executing, "no refusal, no pause");

    // The trip: a claim refused through the executor's clone of the same
    // wallet — the typed `BudgetExhausted` the tool reports the model —
    // also reaches the sink, and the next tick pauses the run.
    assert!(!budget.claim(50), "the refusing claim");
    assert!(budget.tripped(), "the refusal is the recorded event");

    AgentEventSink::emit(
        &sink,
        AgentEvent::Usage {
            call: UsageCall::Answer,
            usage: TokenUsage::new(1, 1),
        },
    )
    .await;
    assert_eq!(
        sink.state(),
        RunState::Paused,
        "a tripped budget pauses the run on the first tick after the trip"
    );
    assert!(
        Journal::open(&run_dir)
            .read()
            .unwrap()
            .iter()
            .any(|event| matches!(
                event,
                RunEvent::Paused {
                    reason: PauseReason::BudgetExhausted
                }
            )),
        "the pause must name the budget — the shared vocabulary, no new reason"
    );
    let record = store.get_run(&run_id).await.unwrap().unwrap();
    assert_eq!(record.status, RunStatus::Paused);

    let _ = fs::remove_dir_all(root);
}

/// The non-regression the latch exists for: a run that downloads *exactly*
/// its budget with nothing refused never pauses. A `consumed >= limit`
/// threshold would stop this run; the latch — the recorded refusal — does
/// not fire.
#[tokio::test]
async fn a_wallet_consumed_exactly_to_its_limit_without_a_refusal_never_pauses() {
    let root = temp_root("download-exact");
    let (store, run_id) = seeded_store(&root, "download-exact", RunStatus::Approved).await;
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).unwrap();
    let budget = DownloadBudget::new(100);
    assert!(
        budget.claim(60) && budget.claim(40),
        "exact fill, none refused"
    );
    assert_eq!(budget.consumed(), 100);
    assert!(!budget.tripped());
    let sink = EngineEventSink::new(
        run_id,
        RunState::Approved,
        Journal::open(&run_dir),
        store,
        SinkBudgets {
            wall_clock: None,
            token_ceiling: None,
            download_budget: Some(budget),
            carried_usage: UsageTotals::default(),
        },
        Instant::now,
    );
    sink.record(TransitionEvent::Begin).await.unwrap();
    AgentEventSink::emit(
        &sink,
        AgentEvent::Usage {
            call: UsageCall::Answer,
            usage: TokenUsage::new(1, 1),
        },
    )
    .await;
    assert_eq!(
        sink.state(),
        RunState::Executing,
        "an exact fill with nothing refused is not a budget exhaustion"
    );
    let _ = fs::remove_dir_all(root);
}

/// The wallet the run did not approve is `None`, and the check is inert —
/// the `token_ceiling: None` pattern. No run that never approved fetch may
/// pause on a download budget that does not exist.
#[tokio::test]
async fn no_armed_download_budget_means_no_download_pause() {
    let root = temp_root("download-none");
    let (store, run_id) = seeded_store(&root, "download-none", RunStatus::Approved).await;
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).unwrap();
    let sink = EngineEventSink::new(
        run_id,
        RunState::Approved,
        Journal::open(&run_dir),
        store,
        SinkBudgets {
            wall_clock: None,
            token_ceiling: None,
            download_budget: None,
            carried_usage: UsageTotals::default(),
        },
        Instant::now,
    );
    sink.record(TransitionEvent::Begin).await.unwrap();
    AgentEventSink::emit(
        &sink,
        AgentEvent::Usage {
            call: UsageCall::Answer,
            usage: TokenUsage::new(1, 1),
        },
    )
    .await;
    assert_eq!(sink.state(), RunState::Executing);
    let _ = fs::remove_dir_all(root);
}

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

/// A resumed sink never re-journals the level its record already holds.
/// The wallet `resume` seeds — 60 of its 100 already spent, journaled by
/// the invocation that spent it — is the baseline the sink journals growth
/// past, so the carried spend stands in the record once. Re-journaling it
/// would double it in the next resume's carry.
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

/// The record holds the spend before the pause that stopped it: the
/// tripping refusal's tick journals the wallet's level first, then pauses.
/// A reader of the record sees the bytes the run claimed, then the budget
/// that stopped it — the shape a resume's carry reads.
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
