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
        question_scope: None,
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

include!("drain_capture_tests/successful_capture.rs");

include!("drain_capture_tests/refusal_failure.rs");

include!("drain_capture_tests/crossed_pairing.rs");

include!("drain_capture_tests/queue_lifecycle.rs");
