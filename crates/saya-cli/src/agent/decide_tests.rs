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

include!("decide_tests/probe_ordering.rs");

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

include!("decide_tests/tie_resolution.rs");

// ===========================================================================
// 5. Cancellation
// ===========================================================================

include!("decide_tests/cancellation.rs");

// ===========================================================================
// 6. Degenerate inputs
// ===========================================================================

include!("decide_tests/candidate_edges.rs");
