//! C2/D12: the capture pairing. A successful agent `bounded_sql_query`
//! completion promotes its pending query AND pairs the FIRST queued capture
//! outcome whose (sql, connection) equals the promoted query's: the capture
//! slot then holds exactly that result — or nothing, never an older one. A
//! refusal clears the slot and records the gap the snapshot names; a failure
//! or a denial never touches the slot; a desynchronised turn promotes no
//! capture. Driven through `drain_stream`, the entry point the loop tick
//! uses.

use super::*;
use crate::agent::tools::{AgentCapture, CaptureRefusalReason};
use crate::interactive::tui::capture::CapturedResult;
use crate::interactive::tui::capture_agent::CaptureGap;
use crate::interactive::tui::types::LastQuery;
use saya_types::{
    EvidenceSource, ExecutionEvidence, ExecutionEvidenceArgs, QueryResult, ResultScope, SqlDialect,
};

fn app_and_state() -> (App, SessionState) {
    (idle_app(), SessionState::new("s1", None, "test-model"))
}

/// A stream that carries exactly `messages`, as the agent thread would.
fn stream_with(messages: Vec<StreamMsg>) -> Stream {
    let (tx, rx) = unbounded_channel();
    for message in messages {
        let _ = tx.send(message);
    }
    Stream {
        rx,
        cancel: CancellationToken::new(),
        prompt: "question".into(),
    }
}

/// A `bounded_sql_query` request event with the given sql and optional
/// connection argument.
fn sql_request(sql: &str, connection: Option<&str>) -> StreamMsg {
    let mut arguments = serde_json::json!({ "sql": sql });
    if let Some(connection) = connection {
        arguments["connection"] = serde_json::json!(connection);
    }
    StreamMsg::Event(AgentEvent::tool_requested(
        "bounded_sql_query",
        arguments,
        None,
    ))
}

/// A completion for `bounded_sql_query`: `summary` decides failure the same
/// way the renderers classify it (the "failed" substring).
fn completed(summary: &str) -> StreamMsg {
    StreamMsg::Event(AgentEvent::ToolCompleted {
        name: "bounded_sql_query".into(),
        summary: summary.into(),
    })
}

fn done() -> StreamMsg {
    StreamMsg::Done(Ok(AgentOutput {
        answer: "the answer".into(),
        events: Vec::new(),
        used_bounded_sql_query: false,
        tool_metadata: Vec::new(),
        usage: TokenUsage::new(0, 0),
        learning_usage: None,
        truncated: false,
        answer_sql: None,
    }))
}

/// One successful agent query's capture: the typed result exactly as the
/// model saw it (connector-capped at the row cap).
fn capture_of(sql: &str, connection: &str, rows: usize) -> AgentCapture {
    AgentCapture {
        sql: sql.to_string(),
        connection: connection.to_string(),
        profile_identity: Some("prof-identity-1".to_string()),
        dialect: SqlDialect::Sqlite,
        result: QueryResult {
            columns: vec!["id".to_string()],
            rows: (0..rows).map(|i| serde_json::json!([i])).collect(),
            row_count: rows,
            truncated: false,
            executed_sql: sql.to_string(),
        },
        row_cap: 100,
        started_unix_ms: 1_700_000_000_000,
        finished_unix_ms: 1_700_000_000_005,
    }
}

/// A previous direct-`/sql` capture, as the slot holds after a /sql success.
fn prev_capture() -> CapturedResult {
    let result = QueryResult {
        columns: vec!["id".to_string()],
        rows: vec![serde_json::json!([0])],
        row_count: 1,
        truncated: false,
        executed_sql: "SELECT 0".to_string(),
    };
    CapturedResult {
        evidence: ExecutionEvidence::for_result(
            &result,
            ExecutionEvidenceArgs {
                execution_id: "xprev-1".to_string(),
                connection_label: "analytics".to_string(),
                connection_identity: None,
                dialect: SqlDialect::Sqlite,
                max_rows: 100,
                started_unix_ms: 1_699_999_999_000,
                finished_unix_ms: 1_699_999_999_005,
                source: EvidenceSource::DirectSql,
            },
        ),
        result,
    }
}

fn selectable_sql(app: &App) -> Option<&str> {
    app.last_query.as_ref().map(|q| q.sql.as_str())
}

fn last_system_line(app: &App) -> String {
    app.transcript
        .blocks()
        .iter()
        .rev()
        .find(|b| b.kind == BlockKind::System)
        .map(|b| b.text.clone())
        .expect("a system block")
}

/// The headline: a successful agent query promotes its pending query AND
/// pairs its own capture — the slot holds exactly the result the model saw,
/// with agent evidence at the model row cap, and one transcript line names
/// the provenance.
#[test]
fn agent_success_captures_exactly_that_result() {
    let (mut app, mut state) = app_and_state();
    let capture = capture_of("SELECT 1", "analytics", 3);
    let expected = capture.result.clone();
    app.request.stream = Some(stream_with(vec![
        sql_request("SELECT 1", Some("analytics")),
        StreamMsg::QueryCaptured(capture),
        completed("3 rows"),
    ]));
    app.drain_stream(&mut state);
    let held = app
        .captured
        .as_ref()
        .expect("the successful agent query's result is captured");
    assert_eq!(
        held.result, expected,
        "the slot holds exactly the result the model saw"
    );
    let evidence = &held.evidence;
    assert_eq!(evidence.source, EvidenceSource::Agent);
    assert_eq!(
        evidence.scope,
        ResultScope::ModelLimited { row_cap: 100 },
        "the agent saw at most the model row cap"
    );
    assert_eq!(evidence.connection_label, "analytics");
    assert_eq!(
        evidence.connection_identity,
        Some("prof-identity-1".to_string()),
        "the profile identity is the capture's"
    );
    assert_eq!(evidence.dialect, SqlDialect::Sqlite);
    assert_eq!(evidence.started_unix_ms, 1_700_000_000_000);
    assert_eq!(evidence.finished_unix_ms, 1_700_000_000_005);
    assert_eq!(evidence.max_rows, 100);
    assert_eq!(evidence.returned_rows, 3);
    let line = last_system_line(&app);
    assert!(
        line.contains("agent: analytics · 3 rows"),
        "the provenance line names source and rows: {line}"
    );
    assert!(
        line.contains("model-limited (first 100 rows)"),
        "the provenance line names the scope: {line}"
    );
}

/// A refusal over the capture budget clears the slot — the previous capture
/// is never silently re-exported as if it were this query's result — and the
/// gap outlives the turn, so a later snapshot still names the reason.
#[test]
fn refused_capture_clears_previous_capture() {
    let (mut app, mut state) = app_and_state();
    app.captured = Some(prev_capture());
    app.request.stream = Some(stream_with(vec![
        sql_request("SELECT 2", Some("analytics")),
        StreamMsg::QueryCaptureRefused {
            sql: "SELECT 2".into(),
            connection: "analytics".into(),
            reason: CaptureRefusalReason::OverBudget,
        },
        completed("1 row"),
    ]));
    app.drain_stream(&mut state);
    assert_eq!(
        selectable_sql(&app),
        Some("SELECT 2"),
        "the successful completion still promotes the selectable query"
    );
    assert!(
        app.captured.is_none(),
        "the refusal clears the slot: no fallback to the older result"
    );
    assert_eq!(app.agent_captures.gap, Some(CaptureGap::OverBudget));
    app.request.stream = Some(stream_with(vec![done()]));
    assert!(app.drain_stream(&mut state), "the turn finished");
    assert_eq!(
        app.agent_captures.gap,
        Some(CaptureGap::OverBudget),
        "the gap outlives the turn: the snapshot may still be asked"
    );
}

/// A failed completion never touches the capture slot: the previous capture
/// survives, nothing is promoted, no gap is recorded.
#[test]
fn failed_agent_query_keeps_previous_capture() {
    let (mut app, mut state) = app_and_state();
    app.captured = Some(prev_capture());
    app.last_query = Some(LastQuery {
        sql: "SELECT 1".into(),
        connection: Some("analytics".into()),
    });
    app.request.stream = Some(stream_with(vec![
        sql_request("SELECT 2", Some("analytics")),
        completed("read-only database tool failed"),
    ]));
    app.drain_stream(&mut state);
    let kept = app
        .captured
        .as_ref()
        .expect("a failed completion never touches the capture slot");
    assert_eq!(kept.result.executed_sql, "SELECT 0", "the previous capture");
    assert_eq!(
        selectable_sql(&app),
        Some("SELECT 1"),
        "a failure never promotes"
    );
    assert_eq!(app.agent_captures.gap, None, "no gap without a promotion");
}

/// Pairing is by content, not arrival order: two queries' captures arrive
/// crossed, and each successful completion still promotes its own capture.
#[test]
fn capture_pairs_by_sql_and_connection_not_order() {
    let (mut app, mut state) = app_and_state();
    let b = capture_of("SELECT 2", "analytics", 2);
    let a = capture_of("SELECT 1", "analytics", 1);
    app.request.stream = Some(stream_with(vec![
        sql_request("SELECT 1", Some("analytics")),
        sql_request("SELECT 2", Some("analytics")),
        StreamMsg::QueryCaptured(b),
        StreamMsg::QueryCaptured(a),
        completed("1 row"),
    ]));
    app.drain_stream(&mut state);
    let held = app
        .captured
        .as_ref()
        .expect("A's completion pairs its capture");
    assert_eq!(
        held.result.executed_sql, "SELECT 1",
        "A's completion took A's capture, though B's arrived first"
    );
    app.request.stream = Some(stream_with(vec![completed("2 rows")]));
    app.drain_stream(&mut state);
    let held = app
        .captured
        .as_ref()
        .expect("B's completion pairs its capture");
    assert_eq!(
        held.result.executed_sql, "SELECT 2",
        "B's completion took B's capture"
    );
}

/// Once the pending FIFO overflows, the turn is desynchronised: no capture
/// is queued, no completion pairs anything, and the capture slot is exactly
/// as it was.
#[test]
fn desync_promotes_no_capture() {
    let (mut app, mut state) = app_and_state();
    app.captured = Some(prev_capture());
    let mut messages: Vec<StreamMsg> = (1..=33)
        .map(|n| sql_request(&format!("SELECT {n}"), Some("analytics")))
        .collect();
    messages.push(StreamMsg::QueryCaptured(capture_of(
        "SELECT 33",
        "analytics",
        1,
    )));
    messages.push(completed("1 row"));
    app.request.stream = Some(stream_with(messages));
    app.drain_stream(&mut state);
    assert!(app.pending_queries_desync, "the overflow desynced the turn");
    let kept = app
        .captured
        .as_ref()
        .expect("a desynchronised turn promotes no capture and clears nothing");
    assert_eq!(kept.result.executed_sql, "SELECT 0");
    assert!(
        app.agent_captures.is_empty(),
        "the capture queue was cleared on desync"
    );
    assert_eq!(app.agent_captures.gap, None);
}

/// R4-9: a TurnReset discards the failed attempt's whole pairing state — the
/// pending FIFO and the unmatched-capture queue — so the retry's completion
/// can only ever promote the retry's own request and pair the retry's own
/// capture, never a stale candidate from the attempt that was thrown away.
#[test]
fn a_turn_reset_discards_the_discarded_attempt_s_pairing_state() {
    let (mut app, mut state) = app_and_state();
    let retry_capture = capture_of("SELECT 2", "analytics", 2);
    let retry_result = retry_capture.result.clone();
    app.request.stream = Some(stream_with(vec![
        // The discarded attempt: its request and its queued capture.
        sql_request("SELECT 1", Some("analytics")),
        StreamMsg::QueryCaptured(capture_of("SELECT 1", "analytics", 1)),
        // The attempt failed mid-stream; the turn retries.
        StreamMsg::Event(AgentEvent::turn_reset()),
        // The retry: its own request, its own capture, its completion.
        sql_request("SELECT 2", Some("analytics")),
        StreamMsg::QueryCaptured(retry_capture),
        completed("2 rows"),
    ]));
    app.drain_stream(&mut state);
    assert_eq!(
        selectable_sql(&app),
        Some("SELECT 2"),
        "the retry's completion promotes the retry's own request, never the \
         discarded attempt's stale candidate"
    );
    let held = app
        .captured
        .as_ref()
        .expect("the retry's completion pairs the retry's own capture");
    assert_eq!(
        held.result, retry_result,
        "the slot holds the retry's capture, never the discarded attempt's"
    );
    assert!(
        app.pending_queries.is_empty(),
        "the retry consumed its own candidate"
    );
}

/// The queue never survives a turn: an unmatched capture (its completion
/// never came) is dropped when the turn ends.
#[test]
fn turn_end_clears_unmatched_captures() {
    let (mut app, mut state) = app_and_state();
    app.request.stream = Some(stream_with(vec![
        sql_request("SELECT 1", Some("analytics")),
        StreamMsg::QueryCaptured(capture_of("SELECT 1", "analytics", 1)),
        done(),
    ]));
    assert!(app.drain_stream(&mut state), "the turn finished");
    assert!(app.captured.is_none(), "no pairing, no capture");
    assert!(
        app.agent_captures.is_empty(),
        "the turn end cleared the unmatched capture"
    );
    assert_eq!(app.agent_captures.gap, None, "no promotion, no gap");
}
