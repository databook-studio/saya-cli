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
