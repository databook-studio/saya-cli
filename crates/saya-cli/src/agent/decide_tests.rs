//! Tests for [`super::decide`].
//!
//! Each numbered behaviour from the task has its own test, driven by a fake
//! [`CandidateExecutor`] that returns scripted results keyed by exact SQL. No
//! database is touched. Probe SQL is built with [`saya_connectors::fanout_probe`]
//! so the exact `joined_rows` / `base_rows` strings the decision layer will run
//! are registered verbatim — the tests assert how the two modules wire together.

use super::{CandidateExecutor, decide};
use async_trait::async_trait;
use saya_agent::CancellationToken;
use saya_connectors::{FanoutProbe, fanout_probe};
use saya_types::{QueryResult, SqlDialect};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// Postgres accepts the most syntax, so it is the default dialect here.
const D: SqlDialect = SqlDialect::Postgres;

/// A scripted executor: exact-SQL lookup returns `Some(result)`; anything else
/// returns `None` (a failed statement). Every call is recorded. If
/// `cancel_after` is set, the token is cancelled once that many calls have
/// completed, so a cancellation test can trip the token mid-decision.
struct ScriptedExecutor {
    results: HashMap<String, QueryResult>,
    calls: Mutex<Vec<String>>,
    cancel_after: Option<(usize, CancellationToken)>,
    first_cancelled: AtomicBool,
}

impl ScriptedExecutor {
    fn new(results: HashMap<String, QueryResult>) -> Self {
        Self {
            results,
            calls: Mutex::new(Vec::new()),
            cancel_after: None,
            first_cancelled: AtomicBool::new(false),
        }
    }

    fn cancel_after(mut self, n: usize, token: CancellationToken) -> Self {
        self.cancel_after = Some((n, token));
        self
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl CandidateExecutor for ScriptedExecutor {
    async fn run(&self, sql: &str) -> Option<QueryResult> {
        let mut calls = self.calls.lock().unwrap();
        calls.push(sql.to_string());
        let count = calls.len();
        drop(calls);
        if let Some((n, token)) = &self.cancel_after
            && count == *n
            && !self.first_cancelled.swap(true, Ordering::SeqCst)
        {
            token.cancel();
        }
        self.results.get(sql).cloned()
    }
}

// ---------------------------------------------------------------------------
// result builders
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

/// A single-cell `COUNT(*) AS n` result, the shape a fan-out probe returns.
fn count_result(n: i64) -> QueryResult {
    rows_result(&["n"], vec![json!([n])])
}

fn nominated(sql: &str) -> Option<String> {
    Some(sql.to_string())
}

/// Register the two probe statements of `probe` with scripted counts so the
/// executor answers them when the decision layer runs them.
fn register_probe(
    results: &mut HashMap<String, QueryResult>,
    probe: &FanoutProbe,
    joined: i64,
    base: i64,
) {
    results.insert(probe.joined_rows.clone(), count_result(joined));
    results.insert(probe.base_rows.clone(), count_result(base));
}

// ===========================================================================
// 1. Execute then tally
// ===========================================================================

#[tokio::test]
async fn execute_then_tally_picks_the_largest_group() {
    // Two runs agree on the same answer; a third disagrees. The vote decides.
    let mut results = HashMap::new();
    results.insert(
        "SELECT a FROM t WHERE id = 1".to_string(),
        rows_result(&["a"], vec![json!([1])]),
    );
    results.insert(
        "SELECT a FROM t WHERE id = 1".to_string(),
        rows_result(&["a"], vec![json!([1])]),
    );
    results.insert(
        "SELECT a FROM t WHERE id = 2".to_string(),
        rows_result(&["a"], vec![json!([2])]),
    );
    let executor = ScriptedExecutor::new(results);

    let decision = decide(
        &[
            nominated("SELECT a FROM t WHERE id = 1"),
            nominated("SELECT a FROM t WHERE id = 1"),
            nominated("SELECT a FROM t WHERE id = 2"),
        ],
        &executor,
        D,
        &CancellationToken::new(),
    )
    .await;

    assert_eq!(decision.winner, Some(0));
    assert_eq!(decision.votes, 2);
    assert_eq!(decision.margin, 1);
    assert!(!decision.tied);
    assert!(!decision.probe_broke_tie);
    // A None nomination and a failed statement both become result: None and
    // take no part in the vote — covered by the degenerate and probe-fail
    // tests below; here every nomination executed.
}

// ===========================================================================
// 2. Deciding `ordered`
// ===========================================================================

#[tokio::test]
async fn ordered_is_set_when_any_candidate_has_top_level_order_by() {
    // Three candidates, all with a top-level ORDER BY. Two return [1,2,3]; the
    // third returns the same rows reversed under a different ORDER BY. Because
    // order is part of the answer, the reversed result is a *different* answer:
    // groups are {0,1}=2 and {2}=1, so votes is 2. Had `ordered` been false,
    // all three would match unordered and votes would be 3. This proves the
    // ordered comparison was used.
    let mut results = HashMap::new();
    results.insert(
        "SELECT a FROM t ORDER BY a".to_string(),
        rows_result(&["a"], vec![json!([1]), json!([2]), json!([3])]),
    );
    results.insert(
        "SELECT a FROM t ORDER BY a".to_string(),
        rows_result(&["a"], vec![json!([1]), json!([2]), json!([3])]),
    );
    results.insert(
        "SELECT a FROM t ORDER BY a DESC".to_string(),
        rows_result(&["a"], vec![json!([3]), json!([2]), json!([1])]),
    );
    let executor = ScriptedExecutor::new(results);

    let decision = decide(
        &[
            nominated("SELECT a FROM t ORDER BY a"),
            nominated("SELECT a FROM t ORDER BY a"),
            nominated("SELECT a FROM t ORDER BY a DESC"),
        ],
        &executor,
        D,
        &CancellationToken::new(),
    )
    .await;

    assert_eq!(decision.winner, Some(0));
    // votes == 2 is the proof: ordered comparison split the reversed result
    // out of the group rather than merging it.
    assert_eq!(decision.votes, 2);
    assert_eq!(decision.margin, 1);
}

#[tokio::test]
async fn ordered_is_not_set_when_order_by_is_only_in_a_subquery() {
    // ORDER BY buried in a derived table is not top-level, so row order is
    // *not* part of the answer. Two candidates return the same rows in a
    // different order and must therefore agree (one group, a winner) rather
    // than tie.
    let mut results = HashMap::new();
    results.insert(
        "SELECT * FROM (SELECT a FROM t ORDER BY a) sub".to_string(),
        rows_result(&["a"], vec![json!([1]), json!([2])]),
    );
    results.insert(
        "SELECT * FROM (SELECT a FROM t ORDER BY a DESC) sub".to_string(),
        rows_result(&["a"], vec![json!([2]), json!([1])]),
    );
    let executor = ScriptedExecutor::new(results);

    let decision = decide(
        &[
            nominated("SELECT * FROM (SELECT a FROM t ORDER BY a) sub"),
            nominated("SELECT * FROM (SELECT a FROM t ORDER BY a DESC) sub"),
        ],
        &executor,
        D,
        &CancellationToken::new(),
    )
    .await;

    assert_eq!(decision.winner, Some(0));
    assert_eq!(decision.votes, 2);
    assert!(!decision.tied);
}

#[tokio::test]
async fn ordered_is_not_set_by_order_by_inside_a_string_constant() {
    // The literal text 'order by' in a string constant must not be mistaken
    // for a clause. No real ORDER BY, so order is not part of the answer and
    // the two differently-ordered results agree.
    let mut results = HashMap::new();
    results.insert(
        "SELECT 'order by' AS s, a FROM t".to_string(),
        rows_result(
            &["s", "a"],
            vec![json!(["order by", 1]), json!(["order by", 2])],
        ),
    );
    results.insert(
        "SELECT 'order by' AS s, a FROM t".to_string(),
        rows_result(
            &["s", "a"],
            vec![json!(["order by", 2]), json!(["order by", 1])],
        ),
    );
    let executor = ScriptedExecutor::new(results);

    let decision = decide(
        &[
            nominated("SELECT 'order by' AS s, a FROM t"),
            nominated("SELECT 'order by' AS s, a FROM t"),
        ],
        &executor,
        D,
        &CancellationToken::new(),
    )
    .await;

    assert_eq!(decision.winner, Some(0));
    assert_eq!(decision.votes, 2);
    assert!(!decision.tied);
}

// ===========================================================================
// 3. No tie: the vote decides (and no probe runs)
// ===========================================================================

#[tokio::test]
async fn no_tie_runs_zero_probe_statements() {
    // A clear winner must not cost any fan-out probe. The executor is called
    // exactly once per nomination and never for a COUNT(*) probe.
    let mut results = HashMap::new();
    results.insert(
        "SELECT a FROM t WHERE id = 1".to_string(),
        rows_result(&["a"], vec![json!([1])]),
    );
    results.insert(
        "SELECT a FROM t WHERE id = 1".to_string(),
        rows_result(&["a"], vec![json!([1])]),
    );
    results.insert(
        "SELECT a FROM t WHERE id = 2".to_string(),
        rows_result(&["a"], vec![json!([2])]),
    );
    let executor = ScriptedExecutor::new(results);

    let decision = decide(
        &[
            nominated("SELECT a FROM t WHERE id = 1"),
            nominated("SELECT a FROM t WHERE id = 1"),
            nominated("SELECT a FROM t WHERE id = 2"),
        ],
        &executor,
        D,
        &CancellationToken::new(),
    )
    .await;

    assert_eq!(decision.winner, Some(0));
    assert!(!decision.tied);
    assert_eq!(decision.fanout, vec![None, None, None]);
    // Exactly three calls — the three nominations. No probe was run.
    assert_eq!(executor.calls().len(), 3);
    assert!(
        !executor.calls().iter().any(|s| s.contains("COUNT(*)")),
        "no probe statement should run when the vote is decisive"
    );
}

// ===========================================================================
// 4. Tie: consult the fan-out evidence
// ===========================================================================

/// Two join queries with a `SUM` whose results disagree, so the vote ties.
/// `fanout_probe` is sound for both. Returns the candidate SQL and their probes.
fn tied_join_candidates() -> (String, String, FanoutProbe, FanoutProbe) {
    let flagged =
        "SELECT SUM(o.amount) AS total FROM orders o JOIN lines l ON l.order_id = o.id".to_string();
    let cleared = "SELECT SUM(o.amount) AS total FROM orders o JOIN lines l ON l.order_id = o.id WHERE o.status = 'open'"
        .to_string();
    let probe_flagged = fanout_probe(&flagged, D).expect("flagged candidate yields a probe");
    let probe_cleared = fanout_probe(&cleared, D).expect("cleared candidate yields a probe");
    (flagged, cleared, probe_flagged, probe_cleared)
}

#[tokio::test]
async fn tie_broken_when_one_group_cleared_and_others_flagged() {
    let (flagged, cleared, probe_flagged, probe_cleared) = tied_join_candidates();

    let mut results = HashMap::new();
    // The two nominated answers disagree, so the vote ties.
    results.insert(flagged.clone(), rows_result(&["total"], vec![json!([100])]));
    results.insert(cleared.clone(), rows_result(&["total"], vec![json!([200])]));
    // Flagged group: join multiplied rows (joined 10 > base 5).
    register_probe(&mut results, &probe_flagged, 10, 5);
    // Cleared group: no fan-out (joined 5 == base 5).
    register_probe(&mut results, &probe_cleared, 5, 5);
    let executor = ScriptedExecutor::new(results);

    let decision = decide(
        &[nominated(&flagged), nominated(&cleared)],
        &executor,
        D,
        &CancellationToken::new(),
    )
    .await;

    // The cleared group (candidate 1) wins; the vote is still reported tied.
    assert_eq!(decision.winner, Some(1));
    assert!(decision.tied);
    assert!(decision.probe_broke_tie);
    assert_eq!(decision.fanout, vec![Some(true), Some(false)]);
}

#[tokio::test]
async fn tie_not_broken_when_no_group_is_cleared() {
    // Both groups flagged: no cleared group, so no winner.
    let (flagged, cleared, probe_flagged, probe_cleared) = tied_join_candidates();
    let mut results = HashMap::new();
    results.insert(flagged.clone(), rows_result(&["total"], vec![json!([100])]));
    results.insert(cleared.clone(), rows_result(&["total"], vec![json!([200])]));
    register_probe(&mut results, &probe_flagged, 10, 5);
    register_probe(&mut results, &probe_cleared, 9, 5); // also flagged
    let executor = ScriptedExecutor::new(results);

    let decision = decide(
        &[nominated(&flagged), nominated(&cleared)],
        &executor,
        D,
        &CancellationToken::new(),
    )
    .await;

    assert_eq!(decision.winner, None);
    assert!(decision.tied);
    assert!(!decision.probe_broke_tie);
    assert_eq!(decision.fanout, vec![Some(true), Some(true)]);
}

#[tokio::test]
async fn tie_not_broken_when_several_groups_are_cleared() {
    // Two cleared groups: ambiguous, so no winner.
    let (flagged, cleared, probe_flagged, probe_cleared) = tied_join_candidates();
    let mut results = HashMap::new();
    results.insert(flagged.clone(), rows_result(&["total"], vec![json!([100])]));
    results.insert(cleared.clone(), rows_result(&["total"], vec![json!([200])]));
    register_probe(&mut results, &probe_flagged, 5, 5); // cleared
    register_probe(&mut results, &probe_cleared, 5, 5); // also cleared
    let executor = ScriptedExecutor::new(results);

    let decision = decide(
        &[nominated(&flagged), nominated(&cleared)],
        &executor,
        D,
        &CancellationToken::new(),
    )
    .await;

    assert_eq!(decision.winner, None);
    assert!(decision.tied);
    assert!(!decision.probe_broke_tie);
    assert_eq!(decision.fanout, vec![Some(false), Some(false)]);
}

#[tokio::test]
async fn tie_not_broken_when_a_probe_statement_fails() {
    // A probe statement that fails to execute is NO SIGNAL — recorded as
    // None, never as fan-out — so it cannot be read as flagged. Here the
    // would-be-cleared group's `base_rows` fails (the known "no such column"
    // case), so its evidence is None and the tie stays unbroken.
    let (flagged, cleared, probe_flagged, probe_cleared) = tied_join_candidates();
    let mut results = HashMap::new();
    results.insert(flagged.clone(), rows_result(&["total"], vec![json!([100])]));
    results.insert(cleared.clone(), rows_result(&["total"], vec![json!([200])]));
    register_probe(&mut results, &probe_flagged, 10, 5); // flagged
    // Cleared group: joined runs, but base_rows is absent from the map so the
    // executor returns None for it — a failed statement, treated as no signal.
    results.insert(probe_cleared.joined_rows.clone(), count_result(5));
    // base_rows deliberately NOT registered → run() returns None.
    let executor = ScriptedExecutor::new(results);

    let decision = decide(
        &[nominated(&flagged), nominated(&cleared)],
        &executor,
        D,
        &CancellationToken::new(),
    )
    .await;

    // The failed probe is None — neither flagged nor cleared — so it must not
    // be read as fan-out, and the tie cannot be broken.
    assert_eq!(decision.fanout, vec![Some(true), None]);
    assert_eq!(decision.winner, None);
    assert!(decision.tied);
    assert!(!decision.probe_broke_tie);
}

#[tokio::test]
async fn tie_not_broken_when_probe_cannot_be_built() {
    // One tied candidate is a plain aggregate with no join, so `fanout_probe`
    // returns None for it. Its evidence is None; even though the other group
    // is cleared, "every other group flagged" fails (this one is None), so the
    // tie stays unbroken.
    let join_sql =
        "SELECT SUM(o.amount) AS total FROM orders o JOIN lines l ON l.order_id = o.id".to_string();
    let plain_sql = "SELECT 1 AS total".to_string();
    let probe_join = fanout_probe(&join_sql, D).expect("join candidate yields a probe");
    assert!(
        fanout_probe(&plain_sql, D).is_none(),
        "plain aggregate has no probe"
    );

    let mut results = HashMap::new();
    results.insert(
        join_sql.clone(),
        rows_result(&["total"], vec![json!([100])]),
    );
    results.insert(
        plain_sql.clone(),
        rows_result(&["total"], vec![json!([200])]),
    );
    register_probe(&mut results, &probe_join, 5, 5); // cleared
    let executor = ScriptedExecutor::new(results);

    let decision = decide(
        &[nominated(&join_sql), nominated(&plain_sql)],
        &executor,
        D,
        &CancellationToken::new(),
    )
    .await;

    assert_eq!(decision.fanout, vec![Some(false), None]);
    assert_eq!(decision.winner, None);
    assert!(decision.tied);
    assert!(!decision.probe_broke_tie);
}

// ===========================================================================
// 5. Cancellation
// ===========================================================================

#[tokio::test]
async fn cancelled_before_any_call_returns_no_winner() {
    let token = CancellationToken::new();
    token.cancel();
    let executor = ScriptedExecutor::new(HashMap::new());

    let decision = decide(
        &[nominated("SELECT 1 AS a"), nominated("SELECT 2 AS a")],
        &executor,
        D,
        &token,
    )
    .await;

    assert_eq!(decision.winner, None);
    assert!(!decision.tied);
    // The check precedes the first executor call, so nothing ran.
    assert!(executor.calls().is_empty());
}

#[tokio::test]
async fn cancelled_before_a_candidate_call_returns_no_winner() {
    // The first candidate runs and the executor cancels the token; the check
    // before the second candidate then returns early.
    let token = CancellationToken::new();
    let mut results = HashMap::new();
    results.insert(
        "SELECT 1 AS a".to_string(),
        rows_result(&["a"], vec![json!([1])]),
    );
    let executor = ScriptedExecutor::new(results).cancel_after(1, token.clone());

    let decision = decide(
        &[nominated("SELECT 1 AS a"), nominated("SELECT 2 AS a")],
        &executor,
        D,
        &token,
    )
    .await;

    assert_eq!(decision.winner, None);
    assert!(!decision.tied);
    // Only the first candidate ran; the second was never reached.
    assert_eq!(executor.calls().len(), 1);
}

#[tokio::test]
async fn cancelled_during_probe_phase_returns_no_winner() {
    // Two tied join candidates. The executor cancels the token during the
    // first probe's joined-rows call; the check before the base-rows call then
    // returns early with no winner, rather than presenting a considered pick.
    let (flagged, cleared, probe_flagged, _probe_cleared) = tied_join_candidates();
    let token = CancellationToken::new();
    let mut results = HashMap::new();
    results.insert(flagged.clone(), rows_result(&["total"], vec![json!([100])]));
    results.insert(cleared.clone(), rows_result(&["total"], vec![json!([200])]));
    // Register only the first probe's joined statement (count call #3).
    results.insert(probe_flagged.joined_rows.clone(), count_result(10));
    let executor = ScriptedExecutor::new(results).cancel_after(3, token.clone());

    let decision = decide(
        &[nominated(&flagged), nominated(&cleared)],
        &executor,
        D,
        &token,
    )
    .await;

    assert_eq!(decision.winner, None);
    // Three calls: two nominations + the first probe's joined count, then the
    // cancellation check before the base count returns early.
    assert_eq!(executor.calls().len(), 3);
}

// ===========================================================================
// 6. Degenerate inputs
// ===========================================================================

#[tokio::test]
async fn empty_slice_returns_without_panicking() {
    let executor = ScriptedExecutor::new(HashMap::new());
    let decision = decide(&[], &executor, D, &CancellationToken::new()).await;

    assert_eq!(decision.winner, None);
    assert_eq!(decision.votes, 0);
    assert!(!decision.tied);
    assert!(executor.calls().is_empty());
}

#[tokio::test]
async fn single_none_nomination_returns_no_winner() {
    let executor = ScriptedExecutor::new(HashMap::new());
    let decision = decide(&[None], &executor, D, &CancellationToken::new()).await;

    assert_eq!(decision.winner, None);
    assert_eq!(decision.votes, 0);
    assert!(executor.calls().is_empty());
}

#[tokio::test]
async fn all_none_nominations_return_no_winner() {
    let executor = ScriptedExecutor::new(HashMap::new());
    let decision = decide(&[None, None, None], &executor, D, &CancellationToken::new()).await;

    assert_eq!(decision.winner, None);
    assert_eq!(decision.votes, 0);
    assert!(executor.calls().is_empty());
}

#[tokio::test]
async fn single_successful_candidate_wins_without_a_probe() {
    let mut results = HashMap::new();
    results.insert(
        "SELECT 1 AS a".to_string(),
        rows_result(&["a"], vec![json!([1])]),
    );
    let executor = ScriptedExecutor::new(results);

    let decision = decide(
        &[nominated("SELECT 1 AS a")],
        &executor,
        D,
        &CancellationToken::new(),
    )
    .await;

    assert_eq!(decision.winner, Some(0));
    assert_eq!(decision.votes, 1);
    assert_eq!(decision.margin, 1);
    assert!(!decision.tied);
    // A single candidate is a decisive vote; no probe runs.
    assert_eq!(executor.calls().len(), 1);
    assert_eq!(decision.fanout, vec![None]);
}
