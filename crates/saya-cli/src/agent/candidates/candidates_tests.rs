//! Tests for the candidate-orchestration core ([`orchestrate`]).
//!
//! Each numbered behaviour from the task has its own test, driven by a scripted
//! [`AttemptRunner`] (returning `AgentOutput`s with nominated SQL, or errors)
//! and a fake [`CandidateExecutor`] keyed by exact SQL — mirroring how
//! `decide_tests.rs` tests the decision layer. No live provider, no database.
//!
//! The orchestration is injectable via [`AttemptRunner`] the way the runtime
//! is injectable via `run_prompt_with_inputs` / `TurnInputs`: the production
//! runner delegates to `run_prompt_with_sink` (which builds `TurnInputs` from
//! config), and these tests inject a scripted runner so the loop, the
//! nomination, the decision, and the `ConsensusDecided` event are assertable in
//! isolation from the runtime (which has its own tests in `runtime_tests.rs`).

use super::super::decide::CandidateExecutor;
use super::super::runtime::AgentRuntimeError;
use super::AttemptRunner;
use async_trait::async_trait;
use saya_agent::{AgentEvent, AgentEventSink, AgentOutput, CancellationToken, TokenUsage};
use saya_types::{QueryResult, SqlDialect};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::orchestrate;

/// Postgres accepts the most syntax, so it is the default dialect here.
const D: SqlDialect = SqlDialect::Postgres;

// ---------------------------------------------------------------------------
// scripted attempt runner
// ---------------------------------------------------------------------------

/// One attempt's scripted outcome: either an `AgentOutput` (carrying nominated
/// SQL) or an error. Stored in a queue and moved out one per `run()` call so
/// the non-`Clone` error type needs no cloning.
enum Outcome {
    Ok(AgentOutput),
    Err(AgentRuntimeError),
}

/// A scripted `AttemptRunner`: returns one queued outcome per call, records
/// every call, and optionally cancels the token once `cancel_after` calls have
/// completed so a cancellation-between-attempts test can trip the token.
struct ScriptedRunner {
    outcomes: Mutex<Vec<Outcome>>,
    calls: AtomicUsize,
    cancel_after: Option<(usize, CancellationToken)>,
}

impl ScriptedRunner {
    fn new(outcomes: Vec<Outcome>) -> Self {
        Self {
            outcomes: Mutex::new(outcomes),
            calls: AtomicUsize::new(0),
            cancel_after: None,
        }
    }

    fn cancel_after(mut self, n: usize, token: CancellationToken) -> Self {
        self.cancel_after = Some((n, token));
        self
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl AttemptRunner for ScriptedRunner {
    fn run(&self) -> Pin<Box<dyn Future<Output = Result<AgentOutput, AgentRuntimeError>> + '_>> {
        Box::pin(async move {
            let count = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if let Some((n, token)) = &self.cancel_after
                && count == *n
            {
                token.cancel();
            }
            let mut outcomes = self.outcomes.lock().expect("outcomes lock");
            match outcomes.len() {
                0 => panic!("ScriptedRunner ran out of outcomes (call #{count})"),
                _ => match outcomes.remove(0) {
                    Outcome::Ok(out) => Ok(out),
                    Outcome::Err(e) => Err(e),
                },
            }
        })
    }
}

// ---------------------------------------------------------------------------
// scripted candidate executor
// ---------------------------------------------------------------------------

/// A scripted `CandidateExecutor`: exact-SQL lookup returns `Some(result)`;
/// anything else returns `None` (a failed statement). Mirrors `ScriptedExecutor`
/// in `decide_tests.rs`.
struct ScriptedExecutor {
    results: HashMap<String, QueryResult>,
}

#[async_trait]
impl CandidateExecutor for ScriptedExecutor {
    async fn run(&self, sql: &str) -> Option<QueryResult> {
        self.results.get(sql).cloned()
    }
}

// ---------------------------------------------------------------------------
// recording sink
// ---------------------------------------------------------------------------

struct RecordingSink {
    events: Arc<Mutex<Vec<AgentEvent>>>,
}

#[async_trait]
impl AgentEventSink for RecordingSink {
    async fn emit(&self, event: AgentEvent) {
        self.events.lock().expect("events lock").push(event);
    }
}

// ---------------------------------------------------------------------------
// result / output builders
// ---------------------------------------------------------------------------

fn rows_result(columns: &[&str], rows: Vec<Value>) -> QueryResult {
    QueryResult {
        columns: columns.iter().map(|s| s.to_string()).collect(),
        rows,
        row_count: 0,
        truncated: false,
        executed_sql: String::new(),
    }
}

/// An `AgentOutput` whose `answer_sql` is `Some(sql)` and whose `answer` names
/// it, so a test can tell the winning attempt's prose from the rest.
fn output_with_sql(sql: &str) -> AgentOutput {
    AgentOutput {
        answer: format!("answer for {sql}"),
        events: Vec::new(),
        used_bounded_sql_query: true,
        tool_metadata: Vec::new(),
        usage: TokenUsage::default(),
        learning_usage: None,
        truncated: false,
        answer_sql: Some(sql.to_string()),
    }
}

/// An `AgentOutput` that designated nothing (`answer_sql: None`).
fn output_without_sql(label: &str) -> AgentOutput {
    AgentOutput {
        answer: format!("answer {label}"),
        events: Vec::new(),
        used_bounded_sql_query: true,
        tool_metadata: Vec::new(),
        usage: TokenUsage::default(),
        learning_usage: None,
        truncated: false,
        answer_sql: None,
    }
}

/// The single `ConsensusDecided` event in `captured`, if any.
fn one_consensus(captured: &[AgentEvent]) -> Option<&AgentEvent> {
    captured
        .iter()
        .find(|e| matches!(e, AgentEvent::ConsensusDecided { .. }))
}

fn consensus_count(captured: &[AgentEvent]) -> usize {
    captured
        .iter()
        .filter(|e| matches!(e, AgentEvent::ConsensusDecided { .. }))
        .count()
}

fn assert_consensus(
    event: &AgentEvent,
    sql: Option<&str>,
    attempts: usize,
    voted: usize,
    votes: usize,
    tied: bool,
) {
    let AgentEvent::ConsensusDecided {
        sql: got_sql,
        attempts: got_attempts,
        voted: got_voted,
        votes: got_votes,
        margin: _,
        tied: got_tied,
        probe_broke_tie: _,
    } = event
    else {
        panic!("expected ConsensusDecided, got {event:?}");
    };
    assert_eq!(*got_attempts, attempts, "attempts: {event:?}");
    assert_eq!(*got_voted, voted, "voted: {event:?}");
    assert_eq!(*got_votes, votes, "votes: {event:?}");
    assert_eq!(*got_tied, tied, "tied: {event:?}");
    match (got_sql.as_deref(), sql) {
        (Some(got), Some(want)) => assert_eq!(got, want, "winning SQL mismatch: {event:?}"),
        (None, None) => {}
        _ => panic!("winning SQL mismatch: {event:?} (expected {sql:?})"),
    }
}

// ===========================================================================
// 1. candidates = 1 runs the agent exactly once and emits no consensus event.
// Prove it by counting, not by inspection.
// ===========================================================================

#[tokio::test]
async fn one_candidate_runs_once_and_emits_no_consensus_event() {
    let runner = ScriptedRunner::new(vec![Outcome::Ok(output_with_sql(
        "SELECT a FROM t WHERE id = 1",
    ))]);
    let executor = ScriptedExecutor {
        results: HashMap::new(),
    };
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
    };

    let out = orchestrate(1, &runner, &executor, D, &sink, &CancellationToken::new())
        .await
        .expect("single attempt succeeds");

    // Proved by counting: exactly one attempt ran …
    assert_eq!(runner.calls(), 1, "the agent ran exactly once");
    // … and no consensus event was emitted (the orchestration never decided).
    assert_eq!(
        consensus_count(&events.lock().expect("events lock")),
        0,
        "no consensus event for a single candidate"
    );
    // The single attempt's output is returned unchanged.
    assert_eq!(
        out.answer_sql.as_deref(),
        Some("SELECT a FROM t WHERE id = 1")
    );
}

// ===========================================================================
// 2. candidates = 3, all attempts agreeing → one event, votes: 3, tied: false,
// and the winning SQL present.
// ===========================================================================

#[tokio::test]
async fn three_candidates_all_agree_emit_one_event_with_three_votes() {
    let sql = "SELECT a FROM t WHERE id = 1";
    let runner = ScriptedRunner::new(vec![
        Outcome::Ok(output_with_sql(sql)),
        Outcome::Ok(output_with_sql(sql)),
        Outcome::Ok(output_with_sql(sql)),
    ]);
    // Every attempt nominated the same SQL, so the executor returns the same
    // result for it — one fingerprint group of three.
    let mut results = HashMap::new();
    results.insert(sql.to_string(), rows_result(&["a"], vec![json!([1])]));
    let executor = ScriptedExecutor { results };
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
    };

    let out = orchestrate(3, &runner, &executor, D, &sink, &CancellationToken::new())
        .await
        .expect("attempts succeed");

    assert_eq!(runner.calls(), 3, "all three attempts ran");
    let captured = events.lock().expect("events lock");
    assert_eq!(consensus_count(&captured), 1, "exactly one consensus event");
    let event = one_consensus(&captured).expect("the consensus event");
    assert_consensus(event, Some(sql), 3, 3, 3, false);
    // The winning attempt's output is returned.
    assert_eq!(out.answer_sql.as_deref(), Some(sql));
}

// ===========================================================================
// 3. candidates = 3 with a disagreement → the majority wins.
// ===========================================================================

#[tokio::test]
async fn three_candidates_disagree_and_the_majority_wins() {
    let majority = "SELECT a FROM t WHERE id = 1";
    let minority = "SELECT a FROM t WHERE id = 2";
    let runner = ScriptedRunner::new(vec![
        Outcome::Ok(output_with_sql(majority)),
        Outcome::Ok(output_with_sql(majority)),
        Outcome::Ok(output_with_sql(minority)),
    ]);
    let mut results = HashMap::new();
    results.insert(majority.to_string(), rows_result(&["a"], vec![json!([1])]));
    results.insert(minority.to_string(), rows_result(&["a"], vec![json!([2])]));
    let executor = ScriptedExecutor { results };
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
    };

    let out = orchestrate(3, &runner, &executor, D, &sink, &CancellationToken::new())
        .await
        .expect("attempts succeed");

    let captured = events.lock().expect("events lock");
    let event = one_consensus(&captured).expect("one consensus event");
    assert_consensus(event, Some(majority), 3, 3, 2, false);
    // The majority attempt's output is returned, not the minority's.
    assert_eq!(out.answer_sql.as_deref(), Some(majority));
}

// ===========================================================================
// 4. attempts that all disagree → event emitted with tied: true and sql: None.
// ===========================================================================

#[tokio::test]
async fn all_attempts_disagree_emit_tied_event_with_no_sql() {
    let a = "SELECT a FROM t WHERE id = 1";
    let b = "SELECT a FROM t WHERE id = 2";
    let c = "SELECT a FROM t WHERE id = 3";
    let runner = ScriptedRunner::new(vec![
        Outcome::Ok(output_with_sql(a)),
        Outcome::Ok(output_with_sql(b)),
        Outcome::Ok(output_with_sql(c)),
    ]);
    let mut results = HashMap::new();
    results.insert(a.to_string(), rows_result(&["a"], vec![json!([1])]));
    results.insert(b.to_string(), rows_result(&["a"], vec![json!([2])]));
    results.insert(c.to_string(), rows_result(&["a"], vec![json!([3])]));
    let executor = ScriptedExecutor { results };
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
    };

    let out = orchestrate(3, &runner, &executor, D, &sink, &CancellationToken::new())
        .await
        .expect("attempts succeed");

    let captured = events.lock().expect("events lock");
    let event = one_consensus(&captured).expect("a consensus event is emitted even with no winner");
    assert_consensus(event, None, 3, 3, 1, true);
    // With no winner, the last successful attempt's output is returned so the
    // caller always has an answer to show.
    assert_eq!(out.answer_sql.as_deref(), Some(c));
}

// ===========================================================================
// 5. one attempt erroring → the others still decide.
// ===========================================================================

#[tokio::test]
async fn one_attempt_erroring_lets_the_others_decide() {
    let sql = "SELECT a FROM t WHERE id = 1";
    let runner = ScriptedRunner::new(vec![
        Outcome::Ok(output_with_sql(sql)),
        Outcome::Err(AgentRuntimeError::Agent("attempt 2 failed".into())),
        Outcome::Ok(output_with_sql(sql)),
    ]);
    let mut results = HashMap::new();
    results.insert(sql.to_string(), rows_result(&["a"], vec![json!([1])]));
    let executor = ScriptedExecutor { results };
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
    };

    let out = orchestrate(3, &runner, &executor, D, &sink, &CancellationToken::new())
        .await
        .expect("the two successful attempts decide");

    assert_eq!(
        runner.calls(),
        3,
        "every attempt ran; the error did not abort the rest"
    );
    let captured = events.lock().expect("events lock");
    // 3 attempts ran, 2 voted (the errored attempt nominated nothing).
    let event = one_consensus(&captured).expect("the survivors still decide");
    assert_consensus(event, Some(sql), 3, 2, 2, false);
    assert_eq!(out.answer_sql.as_deref(), Some(sql));
}

// ===========================================================================
// 6. every attempt erroring → the last error is returned.
// ===========================================================================

#[tokio::test]
async fn every_attempt_erroring_returns_the_last_error() {
    let runner = ScriptedRunner::new(vec![
        Outcome::Err(AgentRuntimeError::Agent("first failure".into())),
        Outcome::Err(AgentRuntimeError::Provider("second failure".into())),
        Outcome::Err(AgentRuntimeError::Database("third failure".into())),
    ]);
    let executor = ScriptedExecutor {
        results: HashMap::new(),
    };
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
    };

    let error = orchestrate(3, &runner, &executor, D, &sink, &CancellationToken::new())
        .await
        .expect_err("all attempts failed");

    assert_eq!(runner.calls(), 3, "every attempt ran before giving up");
    // The last error is surfaced, not the first.
    assert!(
        matches!(error, AgentRuntimeError::Database(ref m) if m == "third failure"),
        "the last error is returned: {error:?}"
    );
    // No consensus event when nothing succeeded.
    assert_eq!(
        consensus_count(&events.lock().expect("events lock")),
        0,
        "no consensus event when every attempt errored"
    );
}

// ===========================================================================
// 7. cancellation between attempts stops early.
// ===========================================================================

#[tokio::test]
async fn cancellation_between_attempts_stops_early() {
    let token = CancellationToken::new();
    let sql = "SELECT a FROM t WHERE id = 1";
    // Three attempts queued, but the runner cancels the token once the second
    // call completes — the check before the third attempt must stop the loop.
    let runner = ScriptedRunner::new(vec![
        Outcome::Ok(output_with_sql(sql)),
        Outcome::Ok(output_with_sql(sql)),
        Outcome::Ok(output_with_sql(sql)),
    ])
    .cancel_after(2, token.clone());
    let mut results = HashMap::new();
    results.insert(sql.to_string(), rows_result(&["a"], vec![json!([1])]));
    let executor = ScriptedExecutor { results };
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
    };

    orchestrate(3, &runner, &executor, D, &sink, &token)
        .await
        .expect("partial run completes");

    // Only two attempts ran: the cancellation between the second and third
    // stopped the loop before the third could start.
    assert_eq!(
        runner.calls(),
        2,
        "cancellation between attempts stopped the loop early (2 of 3 ran)"
    );
}

// ===========================================================================
// Extra: an attempt that succeeds but designates no SQL still runs and
// contributes no nomination — the survivors decide, and the event reports the
// voting attempts honestly. Pins the None-nomination handling end to end.
// ===========================================================================

#[tokio::test]
async fn an_attempt_without_designated_sql_contributes_no_nomination() {
    let sql = "SELECT a FROM t WHERE id = 1";
    let runner = ScriptedRunner::new(vec![
        Outcome::Ok(output_with_sql(sql)),
        Outcome::Ok(output_without_sql("no-designation")),
        Outcome::Ok(output_with_sql(sql)),
    ]);
    let mut results = HashMap::new();
    results.insert(sql.to_string(), rows_result(&["a"], vec![json!([1])]));
    let executor = ScriptedExecutor { results };
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
    };

    let out = orchestrate(3, &runner, &executor, D, &sink, &CancellationToken::new())
        .await
        .expect("attempts succeed");

    let captured = events.lock().expect("events lock");
    let event = one_consensus(&captured).expect("the two designating attempts decide");
    // 3 attempts ran; only 2 nominated (and voted) — the no-designation one
    // neither nominated nor voted.
    assert_consensus(event, Some(sql), 3, 2, 2, false);
    assert_eq!(out.answer_sql.as_deref(), Some(sql));
}
