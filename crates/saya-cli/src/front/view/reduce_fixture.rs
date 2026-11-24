//! The fixtures the reducer's tests fold: one event per thing a session says,
//! and the readers that count a turn's blocks.

use saya_agent::AgentEvent;
use saya_types::{QueryResult, SqlDialect};
use serde_json::json;

use super::{Activity, Entry, TranscriptView, TurnView, apply};
use crate::agent::tools::{AgentCapture, CaptureEvent};
use crate::chat::{ChatEvent, ChatOutcome};

/// The prompt of the demo turn.
pub(super) const PROMPT: &str = "how many orders did we get since December?";

/// The answer the d14 demo settled on, streamed as the model sends it.
pub(super) const ANSWER: [&str; 2] = [
    "110 orders since December 2025 (2025-12-01 through 2026-01-01",
    ", the latest date in the data).",
];

/// One tool call the model made.
pub(super) fn requested(tool: &str, arguments: serde_json::Value) -> ChatEvent {
    ChatEvent::Agent(AgentEvent::ToolRequested {
        name: tool.to_owned(),
        arguments,
        effect: None,
        caller: None,
    })
}

/// The outcome of the call the model made just before.
pub(super) fn completed(tool: &str, summary: &str) -> ChatEvent {
    ChatEvent::Agent(AgentEvent::ToolCompleted {
        name: tool.to_owned(),
        summary: summary.to_owned(),
        caller: None,
    })
}

/// One delta of the answer's text.
pub(super) fn text(delta: &str) -> ChatEvent {
    ChatEvent::Agent(AgentEvent::AssistantText {
        text: delta.to_owned(),
    })
}

/// The typed result one successful query hands the front.
fn capture() -> ChatEvent {
    let result = QueryResult {
        columns: vec![
            "orders_since_december".to_owned(),
            "first_order_date".to_owned(),
            "last_order_date".to_owned(),
        ],
        rows: vec![json!([110, "2025-12-01", "2026-01-01"])],
        row_count: 1,
        truncated: false,
        executed_sql: "SELECT COUNT(*) AS orders_since_december FROM orders".to_owned(),
    };
    ChatEvent::QueryCaptured(CaptureEvent::Captured(AgentCapture {
        sql: result.executed_sql.clone(),
        connection: "demo".to_owned(),
        profile_identity: None,
        dialect: SqlDialect::Sqlite,
        result,
        row_cap: 50,
        started_unix_ms: 0,
        finished_unix_ms: 14_000,
    }))
}

/// The turn ended and the model answered.
pub(super) fn ended() -> ChatEvent {
    ChatEvent::TurnEnded(ChatOutcome::Answered)
}

/// Folds one turn's stream into a fresh transcript, the way a session drives a
/// front: the operator's message, then every event in order.
pub(super) fn fold(events: &[ChatEvent]) -> TranscriptView {
    let mut view = TranscriptView::new();
    view.begin_turn(PROMPT);
    for event in events {
        apply(&mut view, event);
    }
    view
}

/// How many of a turn's entries match.
pub(super) fn count(turn: &TurnView, wanted: impl Fn(&Entry) -> bool) -> usize {
    turn.entries().iter().filter(|entry| wanted(entry)).count()
}

/// The turn's activity block.
pub(super) fn activity(turn: &TurnView) -> &Activity {
    turn.entries()
        .iter()
        .find_map(Entry::activity)
        .expect("a turn with tool calls has one activity block")
}

/// The answer the turn settled on.
pub(super) fn answer(turn: &TurnView) -> &str {
    turn.entries()
        .iter()
        .find_map(|entry| match entry {
            Entry::Answer(answer) => Some(answer.text.as_str()),
            _ => None,
        })
        .expect("a turn the model answered has one answer block")
}

/// The four-step d14 demo turn: schema discovery, three bounded queries, the
/// answer and the result it rests on.
pub(super) fn d14_events() -> Vec<ChatEvent> {
    let mut events = vec![
        requested("schema_discovery", json!({"connection": "demo"})),
        completed("schema_discovery", "discovered 12 tables in demo"),
    ];
    for sql in [
        "SELECT status, COUNT(*) AS n FROM orders GROUP BY status",
        "SELECT order_date, COUNT(*) AS n FROM orders GROUP BY order_date",
        "SELECT COUNT(*) AS orders_since_december FROM orders",
    ] {
        events.push(requested("bounded_sql_query", json!({"sql": sql})));
        events.push(completed("bounded_sql_query", "1 row"));
    }
    events.extend([text(ANSWER[0]), text(ANSWER[1]), capture(), ended()]);
    events
}
