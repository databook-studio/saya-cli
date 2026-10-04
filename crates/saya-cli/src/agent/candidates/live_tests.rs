//! Source-owner proof for the live candidate vote: when the question's elapsed
//! ceiling latches a budget stop mid-vote, production [`orchestrate`] keeps the
//! live vote owned — the real connector execution is awaited through source
//! return (never dropped mid-flight), the delegated `request_cancel` runs on the
//! real connection, and no later query, probe, or consensus may begin.
//!
//! The executor is the REAL [`LiveCandidateExecutor`] built as a struct literal
//! over the delegate-only [`trace::TracedSqlite`]; the slow source is the
//! canonical recursive SELECT, and the stop comes from a real
//! [`QuestionScope`] whose elapsed ceiling is the minimum (1s).

#[path = "live_trace_tests.rs"]
mod trace;

#[path = "live_support_tests.rs"]
mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use saya_agent::{AgentEvent, BudgetDimension, CancellationToken};
use saya_connectors::{CancelRequestOutcome, ConnectorOptions, DatabaseConnector, SqliteConnector};
use saya_types::SqlDialect;
use tokio::sync::{Mutex, oneshot};

use super::super::orchestrate::orchestrate;
use super::LiveCandidateExecutor;
use crate::answering_receipts::AnsweringReceiptSink;
use support::{RecordingSink, ScriptedRunner, nominating_output, one_second_scope};
use trace::{ExecutionTrace, FIXTURE_ID, SLOW, TracedSqlite};

#[tokio::test]
async fn a_budget_stopped_vote_stays_owned_until_its_real_source_settles() {
    let root = std::env::temp_dir().join(format!(
        "saya-live-vote-{}-{}",
        std::process::id(),
        FIXTURE_ID.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).expect("isolated live-vote fixture directory");
    let database = root.join("live-vote.sqlite3");
    let seed = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&database)
            .create_if_missing(true),
    )
    .await
    .expect("synthetic SQLite database creates");
    seed.close().await;
    let connector = SqliteConnector::open(
        &database,
        false,
        ConnectorOptions {
            query_timeout_seconds: 5,
            ..Default::default()
        },
    )
    .await
    .expect("real SQLite connector opens");
    connector.connect().await.expect("real SQLite connects");

    let (first_pending, mut started) = oneshot::channel();
    let trace = Arc::new(ExecutionTrace {
        first_pending: Mutex::new(Some(first_pending)),
        settled: std::sync::atomic::AtomicBool::new(false),
        cancel_calls: AtomicUsize::new(0),
        execute_calls: AtomicUsize::new(0),
        cancel_outcome: Mutex::new(None),
    });
    let scope = one_second_scope(CancellationToken::new());
    // The production executor hears the question's own token, the way
    // `build_executor` wires it.
    let live_executor = LiveCandidateExecutor {
        connector: Box::new(TracedSqlite {
            inner: connector,
            trace: Arc::clone(&trace),
        }),
        max_rows: 1,
        cancellation: scope.cancellation(),
    };
    let cancellation = scope.cancellation();
    let runner = ScriptedRunner::new(vec![
        nominating_output("first candidate settles on the recursive count", SLOW),
        nominating_output("second candidate settles on the same recursive count", SLOW),
    ]);
    let sink = Arc::new(RecordingSink {
        events: std::sync::Mutex::new(Vec::new()),
    });
    // The real orchestrate future runs as its own local task (its runner
    // borrows make it non-Send, exactly as in production) so the test can
    // watch the trace's first-Pending signal while that same future is still
    // owned and pending — the signal is sent from inside the vote's real
    // execute.
    let local = tokio::task::LocalSet::new();
    let task = local.spawn_local({
        let sink = Arc::clone(&sink);
        async move {
            let receipts = AnsweringReceiptSink::new(sink.as_ref());
            orchestrate(
                2,
                &runner,
                &live_executor,
                SqlDialect::Sqlite,
                &receipts,
                &cancellation,
                &scope,
            )
            .await
        }
    });
    let outcome = local
        .run_until(async move {
            // The vote really started: the real source went Pending while the
            // same orchestrate future was still owned and pending.
            tokio::time::timeout(Duration::from_secs(5), &mut started)
                .await
                .expect("the delegated execute reached Pending before the vote settled")
                .expect("the traced source stayed connected until it signalled");
            tokio::time::timeout(Duration::from_secs(15), task)
                .await
                .expect("the orchestrated vote settles within 15s")
                .expect("the orchestrate task joins without panicking")
        })
        .await;
    let outcome = outcome.expect("the stopped vote returns a retained answer");
    // The retain-vs-drop oracle: when the outer orchestrate future returns, the
    // inner execute future has been awaited to Ready — a dropped vote would
    // leave the source abandoned mid-flight with `settled` never stored.
    assert!(
        trace.settled.load(Ordering::Acquire),
        "the live vote is owned to source return: the inner execute settled"
    );
    // The delegated `request_cancel` ran on the real connection, ordering
    // guaranteed: the owner's join cannot complete before it returns.
    assert!(
        trace.cancel_calls.load(Ordering::Acquire) >= 1,
        "the stop delegates a real request_cancel through the connector"
    );
    assert_eq!(
        *trace.cancel_outcome.lock().await,
        Some(CancelRequestOutcome::LocalInterruptRequested),
        "the interrupt is the real connector's answer"
    );
    // Exactly one query started (the vote's first nomination); the second
    // nomination early-returns and no probe ever begins.
    assert_eq!(
        trace.execute_calls.load(Ordering::Acquire),
        1,
        "no later query or probe starts after the stop"
    );
    assert!(
        !sink
            .events
            .lock()
            .expect("events lock")
            .iter()
            .any(|event| matches!(event, AgentEvent::ConsensusDecided { .. })),
        "no winning consensus is fabricated over a stopped vote"
    );
    // The earlier answer and its designation evidence are retained, and the
    // stop is carried truthfully: partial, with exactly one typed event.
    assert_eq!(
        outcome.answer, "second candidate settles on the same recursive count",
        "the last settled attempt's answer is retained"
    );
    assert_eq!(
        outcome.answer_sql.as_deref(),
        Some(SLOW),
        "the nomination is retained"
    );
    assert!(
        outcome
            .events
            .iter()
            .any(|event| matches!(event, AgentEvent::AnswerDesignated { .. })),
        "the earlier designation evidence survives: {:?}",
        outcome.events
    );
    assert!(
        outcome.truncated,
        "the retained answer is partial, not a natural completion"
    );
    let stops = outcome
        .events
        .iter()
        .filter(|event| {
            matches!(
                event,
                AgentEvent::AnsweringBudgetStopped {
                    dimension: BudgetDimension::ElapsedTime
                }
            )
        })
        .count();
    assert_eq!(
        stops, 1,
        "exactly one typed elapsed-time stop: {:?}",
        outcome.events
    );
    let _ = std::fs::remove_dir_all(root);
}
