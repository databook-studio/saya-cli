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
use super::super::question_scope::QuestionScope;
use super::super::runtime::AgentRuntimeError;
use super::AttemptRunner;
use crate::{answering_receipts::AnsweringReceiptSink, config::runtime::RuntimeConfig};
use async_trait::async_trait;
use saya_agent::{AgentEvent, AgentEventSink, AgentOutput, CancellationToken, TokenUsage};
use saya_config::{ConnectionsFile, ResolutionInput, resolve};
use saya_types::{QueryResult, SqlDialect};
use serde_json::{Value, json};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
};

use super::orchestrate;

/// Postgres accepts the most syntax, so it is the default dialect here.
const D: SqlDialect = SqlDialect::Postgres;

use saya_agent::{BudgetDimension, CancellationReason};

fn test_scope(cancellation: CancellationToken) -> QuestionScope {
    let connections = ConnectionsFile::default();
    let runtime = RuntimeConfig {
        resolved: resolve(ResolutionInput::new(connections.clone())).expect("test config resolves"),
        connections,
        config_path: None,
        connections_path: None,
        cache_scope: PathBuf::from("/tmp/saya-candidate-tests"),
        investigations_root: PathBuf::from("/tmp/saya-candidate-tests/investigations"),
        secret_values: BTreeMap::new(),
    };
    QuestionScope::new(&runtime, cancellation)
}

async fn orchestrate_test(
    candidates: usize,
    runner: &dyn AttemptRunner,
    executor: &dyn CandidateExecutor,
    sink: &RecordingSink,
    cancellation: &CancellationToken,
) -> Result<AgentOutput, AgentRuntimeError> {
    let scope = test_scope(cancellation.clone());
    let receipts = AnsweringReceiptSink::new(sink);
    orchestrate(
        candidates,
        runner,
        executor,
        D,
        &receipts,
        cancellation,
        &scope,
    )
    .await
}

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
/// every call, and optionally latches the token with a stop reason once
/// `cancel_after` calls have completed so a stop-between-attempts test can
/// fire the token exactly where a real budget or signal would.
struct ScriptedRunner {
    outcomes: Mutex<Vec<Outcome>>,
    calls: AtomicUsize,
    cancel_after: Option<(usize, CancellationToken, CancellationReason)>,
}

impl ScriptedRunner {
    fn new(outcomes: Vec<Outcome>) -> Self {
        Self {
            outcomes: Mutex::new(outcomes),
            calls: AtomicUsize::new(0),
            cancel_after: None,
        }
    }

    fn cancel_after(self, n: usize, token: CancellationToken) -> Self {
        self.cancel_after_with(n, token, CancellationReason::User)
    }

    fn cancel_after_with(
        mut self,
        n: usize,
        token: CancellationToken,
        reason: CancellationReason,
    ) -> Self {
        self.cancel_after = Some((n, token, reason));
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
            if let Some((n, token, reason)) = &self.cancel_after
                && count == *n
            {
                token.cancel_with_reason(*reason);
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

/// A `CandidateExecutor` that only counts calls and returns nothing. The vote
/// segment is the only orchestrate path that calls the executor, so a zero
/// count is the proof that no vote segment ever began.
struct CountingExecutor {
    calls: AtomicUsize,
}

impl CountingExecutor {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl CandidateExecutor for CountingExecutor {
    async fn run(&self, _: &str) -> Option<QueryResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        None
    }
}

/// A scripted `CandidateExecutor` that serves its result and latches the
/// question token with a stop reason on every call — the verification work
/// itself is where the stop fired, the way a deadline expiring mid-decision
/// would.
struct StopDuringVoteExecutor {
    calls: AtomicUsize,
    results: HashMap<String, QueryResult>,
    reason: CancellationReason,
    token: CancellationToken,
}

impl StopDuringVoteExecutor {
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl CandidateExecutor for StopDuringVoteExecutor {
    async fn run(&self, sql: &str) -> Option<QueryResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.token.cancel_with_reason(self.reason);
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

/// An `AgentOutput` whose turn settled on a clarification: the loop emitted
/// `ClarificationNeeded` and completed, so the returned Ok output carrying the
/// event is a settled pause — the way `loop_runner/clarification.rs` builds it.
fn clarification_output(sql: &str) -> AgentOutput {
    let mut out = output_with_sql(sql);
    out.events.push(AgentEvent::clarification_needed(
        "which definition of \"active users\" should I use?",
        vec![
            "sessions in the last 30 days".into(),
            "purchases in the last 90 days".into(),
        ],
    ));
    out
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

fn output_with_usage(sql: &str, input: u64, output: u64) -> AgentOutput {
    let mut result = output_with_sql(sql);
    result.usage = TokenUsage::new(input, output);
    result
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

    let out = orchestrate_test(1, &runner, &executor, &sink, &CancellationToken::new())
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

    let out = orchestrate_test(3, &runner, &executor, &sink, &CancellationToken::new())
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

    let out = orchestrate_test(3, &runner, &executor, &sink, &CancellationToken::new())
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

    let out = orchestrate_test(3, &runner, &executor, &sink, &CancellationToken::new())
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

    let out = orchestrate_test(3, &runner, &executor, &sink, &CancellationToken::new())
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

    let error = orchestrate_test(3, &runner, &executor, &sink, &CancellationToken::new())
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

    orchestrate_test(3, &runner, &executor, &sink, &token)
        .await
        .expect("partial run completes");

    // Only two attempts ran: the cancellation between the second and third
    // stopped the loop before the third could start.
    assert_eq!(
        runner.calls(),
        2,
        "cancellation between attempts stopped the loop early (2 of 3 ran)"
    );
    assert_eq!(
        consensus_count(&events.lock().expect("events lock")),
        0,
        "a stopped question never starts vote or consensus work"
    );
}

#[tokio::test]
async fn selected_candidate_retains_all_settled_attempt_usage() {
    let sql = "SELECT a FROM t WHERE id = 1";
    let runner = ScriptedRunner::new(vec![
        Outcome::Ok(output_with_usage(sql, 2, 3)),
        Outcome::Ok(output_with_usage(sql, 5, 7)),
    ]);
    let executor = ScriptedExecutor {
        results: HashMap::from([(sql.to_string(), rows_result(&["a"], vec![json!([1])]))]),
    };
    let sink = RecordingSink {
        events: Arc::new(Mutex::new(Vec::new())),
    };

    let cancellation = CancellationToken::new();
    let scope = test_scope(cancellation.clone());
    let receipts = AnsweringReceiptSink::new(&sink);
    receipts
        .emit(AgentEvent::usage(
            saya_agent::UsageCall::Answer,
            TokenUsage::new(2, 3),
        ))
        .await;
    receipts
        .emit(AgentEvent::usage(
            saya_agent::UsageCall::Answer,
            TokenUsage::new(5, 7),
        ))
        .await;
    let output = orchestrate(2, &runner, &executor, D, &receipts, &cancellation, &scope)
        .await
        .expect("two settled candidates decide");

    assert_eq!(
        output.usage.input_tokens, 7,
        "both paid input receipts survive"
    );
    assert_eq!(
        output.usage.output_tokens, 10,
        "both paid output receipts survive"
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

    let out = orchestrate_test(3, &runner, &executor, &sink, &CancellationToken::new())
        .await
        .expect("attempts succeed");

    let captured = events.lock().expect("events lock");
    let event = one_consensus(&captured).expect("the two designating attempts decide");
    // 3 attempts ran; only 2 nominated (and voted) — the no-designation one
    // neither nominated nor voted.
    assert_consensus(event, Some(sql), 3, 2, 2, false);
    assert_eq!(out.answer_sql.as_deref(), Some(sql));
}

// ===========================================================================
// A settled clarification stops candidates: once an attempt's returned output
// ended on a settled ClarificationNeeded, no further attempt may launch, the
// question must not enter verification/vote (the executor is never called —
// the vote segment is the only path that calls it), and no consensus may be
// fabricated for a paused turn. The paused output itself is the answer.
// ===========================================================================

#[tokio::test]
async fn a_settled_clarification_stops_further_attempts_and_never_begins_a_vote_segment() {
    let first = "SELECT a FROM t WHERE id = 1";
    let second = "SELECT a FROM t WHERE id = 2";
    // Attempt 1 settles on a clarification; attempts 2 and 3 are queued and
    // observable (by the runner's call count) if the loop ever starts them.
    let runner = ScriptedRunner::new(vec![
        Outcome::Ok(clarification_output(first)),
        Outcome::Ok(output_with_sql(second)),
        Outcome::Ok(output_with_sql(second)),
    ]);
    let executor = CountingExecutor::new();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
    };

    let out = orchestrate_test(3, &runner, &executor, &sink, &CancellationToken::new())
        .await
        .expect("the settled clarification is a finished outcome, not an error");

    assert_eq!(
        runner.calls(),
        1,
        "attempt 2 never started once the clarification settled"
    );
    assert_eq!(
        executor.calls(),
        0,
        "no vote segment began: the executor is never called"
    );
    assert_eq!(
        consensus_count(&events.lock().expect("events lock")),
        0,
        "no consensus may be fabricated for a paused question"
    );
    // The returned output IS the paused attempt's, unchanged.
    assert_eq!(out.answer_sql.as_deref(), Some(first));
    assert!(
        out.events
            .iter()
            .any(|event| matches!(event, AgentEvent::ClarificationNeeded { .. })),
        "the paused attempt's evidence is preserved: {:?}",
        out.events
    );
    assert!(
        !out.truncated,
        "a settled clarification is not a truncation: {:?}",
        out.truncated
    );
}

// ===========================================================================
// A settled clarification after an errored attempt: the error did not stop the
// loop, the clarification does — and the returned paused output keeps the
// earlier paid usage summed truthfully.
// ===========================================================================

#[tokio::test]
async fn a_settled_clarification_after_an_errored_attempt_returns_the_paused_output_with_paid_usage()
 {
    let sql = "SELECT a FROM t WHERE id = 1";
    let runner = ScriptedRunner::new(vec![
        Outcome::Err(AgentRuntimeError::Agent("attempt 1 failed".into())),
        Outcome::Ok(clarification_output(sql)),
    ]);
    let executor = CountingExecutor::new();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
    };

    let cancellation = CancellationToken::new();
    let scope = test_scope(cancellation.clone());
    let receipts = AnsweringReceiptSink::new(&sink);
    receipts
        .emit(AgentEvent::usage(
            saya_agent::UsageCall::Answer,
            TokenUsage::new(4, 2),
        ))
        .await;
    let out = orchestrate(2, &runner, &executor, D, &receipts, &cancellation, &scope)
        .await
        .expect("the settled clarification is returned");

    assert_eq!(
        runner.calls(),
        2,
        "the errored attempt did not stop the loop; the clarification did"
    );
    assert_eq!(
        executor.calls(),
        0,
        "no vote segment began: the executor is never called"
    );
    assert_eq!(
        consensus_count(&events.lock().expect("events lock")),
        0,
        "no consensus may be fabricated for a paused question"
    );
    assert_eq!(out.answer_sql.as_deref(), Some(sql));
    assert!(
        out.events
            .iter()
            .any(|event| matches!(event, AgentEvent::ClarificationNeeded { .. })),
        "the paused attempt's evidence is preserved: {:?}",
        out.events
    );
    assert_eq!(
        out.usage.input_tokens, 4,
        "the earlier paid usage survives on the paused output"
    );
    assert_eq!(out.usage.output_tokens, 2, "same, for output tokens");
    assert!(
        !out.truncated,
        "a settled clarification is not a truncation: {:?}",
        out.truncated
    );
}

// ===========================================================================
// A budget stop between attempts: the stop-return keeps the last settled
// answer, but the retained output must carry the actual stop — partial
// (`truncated: true`) with the typed `AnsweringBudgetStopped` event naming the
// real dimension — not come back looking like a natural completion. Before the
// repair the returned output has `truncated: false` and no budget-stop event.
// ===========================================================================

#[tokio::test]
async fn a_budget_stop_before_verification_marks_the_retained_output_partial_with_a_typed_event() {
    let token = CancellationToken::new();
    let sql = "SELECT a FROM t WHERE id = 1";
    // Attempt 1 settles naturally; the elapsed budget latches the token the
    // moment it completes — before any verification could begin.
    let runner = ScriptedRunner::new(vec![
        Outcome::Ok(output_with_sql(sql)),
        Outcome::Ok(output_with_sql(sql)),
    ])
    .cancel_after_with(
        1,
        token.clone(),
        CancellationReason::Budget(BudgetDimension::ElapsedTime),
    );
    let mut results = HashMap::new();
    results.insert(sql.to_string(), rows_result(&["a"], vec![json!([1])]));
    let executor = ScriptedExecutor { results };
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
    };

    let out = orchestrate_test(2, &runner, &executor, &sink, &token)
        .await
        .expect("the retained answer returns");

    assert_eq!(
        runner.calls(),
        1,
        "the stop between attempts kept attempt 2 from starting"
    );
    assert!(
        out.truncated,
        "the retained answer is partial, not a natural completion"
    );
    assert!(
        out.events.iter().any(|event| matches!(
            event,
            AgentEvent::AnsweringBudgetStopped {
                dimension: BudgetDimension::ElapsedTime
            }
        )),
        "the retained events carry the typed budget stop for the real dimension: {:?}",
        out.events
    );
    assert_eq!(
        consensus_count(&events.lock().expect("events lock")),
        0,
        "no consensus may be fabricated over a stopped run"
    );
}

// ===========================================================================
// A budget stop during the vote's verification work: the decision dies with
// the stop, the consensus emit is skipped, and the retained answer comes back
// partial with the typed event — not as a natural completion.
// ===========================================================================

#[tokio::test]
async fn a_budget_stop_during_the_vote_returns_the_answer_as_partial_with_no_consensus() {
    let token = CancellationToken::new();
    let sql = "SELECT a FROM t WHERE id = 1";
    let runner = ScriptedRunner::new(vec![
        Outcome::Ok(output_with_sql(sql)),
        Outcome::Ok(output_with_sql(sql)),
    ]);
    let mut results = HashMap::new();
    results.insert(sql.to_string(), rows_result(&["a"], vec![json!([1])]));
    let executor = StopDuringVoteExecutor {
        calls: AtomicUsize::new(0),
        results,
        reason: CancellationReason::Budget(BudgetDimension::ElapsedTime),
        token: token.clone(),
    };
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
    };

    let out = orchestrate_test(2, &runner, &executor, &sink, &token)
        .await
        .expect("the retained answer returns");

    assert_eq!(runner.calls(), 2, "both attempts settled naturally");
    assert!(executor.calls() >= 1, "the vote ran its verification work");
    assert!(
        out.truncated,
        "the retained answer is partial, not a natural completion"
    );
    assert!(
        out.events.iter().any(|event| matches!(
            event,
            AgentEvent::AnsweringBudgetStopped {
                dimension: BudgetDimension::ElapsedTime
            }
        )),
        "the retained events carry the typed budget stop: {:?}",
        out.events
    );
    assert_eq!(
        consensus_count(&events.lock().expect("events lock")),
        0,
        "a stopped question never emits a consensus"
    );
}

// ===========================================================================
// Green-side pins for the retained-output stop rules. (These cannot be red:
// two of them pass by accident on the un-repaired code, which never touched
// the output; they pin what the repair must keep true.)
// ===========================================================================

/// An `AgentOutput` the answering loop itself budget-stopped: partial, with
/// its own typed event already in the retained events.
fn budget_stopped_output(sql: &str) -> AgentOutput {
    let mut out = output_with_sql(sql);
    out.truncated = true;
    out.events.push(AgentEvent::answering_budget_stopped(
        BudgetDimension::ElapsedTime,
    ));
    out
}

/// An `AgentOutput` already truncated for a non-budget reason — the legacy
/// truncation shape (e.g. a salvaged partial from a truncated provider
/// response): partial, no typed budget event.
fn legacy_truncated_output(sql: &str) -> AgentOutput {
    let mut out = output_with_sql(sql);
    out.truncated = true;
    out
}

/// A loop budget-stop is kept once: the token's Budget reason must not append
/// a second typed event to an output that already carries its own.
#[tokio::test]
async fn a_retained_output_already_budget_stopped_is_not_marked_twice() {
    let token = CancellationToken::new();
    let sql = "SELECT a FROM t WHERE id = 1";
    let runner = ScriptedRunner::new(vec![
        Outcome::Ok(budget_stopped_output(sql)),
        Outcome::Ok(output_with_sql(sql)),
    ])
    .cancel_after_with(
        1,
        token.clone(),
        CancellationReason::Budget(BudgetDimension::ElapsedTime),
    );
    let mut results = HashMap::new();
    results.insert(sql.to_string(), rows_result(&["a"], vec![json!([1])]));
    let executor = ScriptedExecutor { results };
    let sink = RecordingSink {
        events: Arc::new(Mutex::new(Vec::new())),
    };

    let out = orchestrate_test(2, &runner, &executor, &sink, &token)
        .await
        .expect("the retained answer returns");

    assert!(out.truncated, "the loop's own partial marking stands");
    let stops = out
        .events
        .iter()
        .filter(|event| matches!(event, AgentEvent::AnsweringBudgetStopped { .. }))
        .count();
    assert_eq!(
        stops, 1,
        "the loop's budget-stop event is kept, not duplicated: {:?}",
        out.events
    );
}

/// A legacy truncation is never relabelled as a configured-budget stop: the
/// token's Budget reason must not fabricate a typed event onto an output that
/// was already truncated for some other reason.
#[tokio::test]
async fn a_legacy_truncated_output_is_not_relabelled_as_a_budget_stop() {
    let token = CancellationToken::new();
    let sql = "SELECT a FROM t WHERE id = 1";
    let runner = ScriptedRunner::new(vec![
        Outcome::Ok(legacy_truncated_output(sql)),
        Outcome::Ok(output_with_sql(sql)),
    ])
    .cancel_after_with(
        1,
        token.clone(),
        CancellationReason::Budget(BudgetDimension::ElapsedTime),
    );
    let mut results = HashMap::new();
    results.insert(sql.to_string(), rows_result(&["a"], vec![json!([1])]));
    let executor = ScriptedExecutor { results };
    let sink = RecordingSink {
        events: Arc::new(Mutex::new(Vec::new())),
    };

    let out = orchestrate_test(2, &runner, &executor, &sink, &token)
        .await
        .expect("the retained answer returns");

    assert!(out.truncated, "the legacy truncation stands");
    assert!(
        !out.events
            .iter()
            .any(|event| matches!(event, AgentEvent::AnsweringBudgetStopped { .. })),
        "a legacy truncation is not relabelled as a configured-budget stop: {:?}",
        out.events
    );
}

/// A User stop leaves the retained output untouched — no partial marking, no
/// typed event; the ask adapter classifies a user stop from the token itself.
#[tokio::test]
async fn a_user_stop_leaves_the_retained_output_untouched() {
    let token = CancellationToken::new();
    let sql = "SELECT a FROM t WHERE id = 1";
    let runner = ScriptedRunner::new(vec![
        Outcome::Ok(output_with_sql(sql)),
        Outcome::Ok(output_with_sql(sql)),
    ])
    .cancel_after(1, token.clone());
    let mut results = HashMap::new();
    results.insert(sql.to_string(), rows_result(&["a"], vec![json!([1])]));
    let executor = ScriptedExecutor { results };
    let sink = RecordingSink {
        events: Arc::new(Mutex::new(Vec::new())),
    };

    let out = orchestrate_test(2, &runner, &executor, &sink, &token)
        .await
        .expect("the retained answer returns");

    assert!(
        !out.truncated,
        "a user stop does not mark the answer partial"
    );
    assert!(
        !out.events
            .iter()
            .any(|event| matches!(event, AgentEvent::AnsweringBudgetStopped { .. })),
        "a user stop never fabricates a budget-stop event: {:?}",
        out.events
    );
}
