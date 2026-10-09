//! S5/D7: only a successful, concrete `bounded_sql_query` completion becomes
//! the latest selectable query. A request is a candidate on a bounded FIFO;
//! the completion decides promotion; a failure or a denial never promotes;
//! fan-out never becomes selectable. Driven through `drain_stream`, the same
//! entry point the loop tick uses.

use super::*;

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

/// A `bounded_sql_query_all` request event.
fn fanout_request(sql: &str) -> StreamMsg {
    StreamMsg::Event(AgentEvent::tool_requested(
        "bounded_sql_query_all",
        serde_json::json!({ "sql": sql }),
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

/// A completion for `bounded_sql_query_all`.
fn fanout_completed(summary: &str) -> StreamMsg {
    StreamMsg::Event(AgentEvent::ToolCompleted {
        name: "bounded_sql_query_all".into(),
        summary: summary.into(),
    })
}

/// A denial of `bounded_sql_query`: no `ToolCompleted` follows it.
fn denied() -> StreamMsg {
    StreamMsg::Event(AgentEvent::ToolDenied {
        name: "bounded_sql_query".into(),
        reason: "user denied the call".into(),
    })
}

fn selectable_sql(app: &App) -> Option<&str> {
    app.last_query.as_ref().map(|q| q.sql.as_str())
}

fn selectable_connection(app: &App) -> Option<&str> {
    app.last_query
        .as_ref()
        .and_then(|q| q.connection.as_deref())
}
#[path = "drain_selectable_tests/consumption.rs"]
mod consumption;
#[path = "drain_selectable_tests/overflow.rs"]
mod overflow;
#[path = "drain_selectable_tests/promotion.rs"]
mod promotion;
#[path = "drain_selectable_tests/selection.rs"]
mod selection;
