//! The four database-golden scenarios. Each drives the real REPL against a
//! scripted provider and a SQLite fixture, then records what happened.
//!
//! The scripts are the model's side: one request slot per script step, in
//! order. A scenario asserts the behaviour it exists to pin (the calls the
//! model made, the answer or stop the user saw) *before* comparing the
//! golden, so an update run cannot quietly bless a scenario that never ran.

use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::harness::{self, ScriptStep};
use super::record::{Golden, assert_golden};
use super::setup::{ANSWER, QUESTION, assert_last_state, fixture, settle, spawn, visible_states};

fn listener() -> TcpListener {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    listener
}

fn record(name: &str, observed: &super::setup::Observed) -> Golden {
    Golden::build(
        name,
        &observed.bodies,
        &observed.served,
        &observed.normaliser,
        visible_states(&observed.stdout, &observed.stderr),
    )
}

#[test]
fn a_plain_question_discovers_then_queries_then_answers() {
    let fixture = fixture("plain");
    let mut server = harness::spawn_script_server(
        listener(),
        vec![
            ScriptStep::ToolCall {
                name: "schema_discovery".into(),
                arguments: serde_json::json!({}),
            },
            ScriptStep::ToolCall {
                name: "bounded_sql_query".into(),
                arguments: serde_json::json!({
                    "sql": "SELECT SUM(amount) AS total FROM revenue"
                }),
            },
            ScriptStep::Natural,
        ],
        Arc::new(AtomicBool::new(false)),
    );
    let mut child = spawn(&fixture, &server.address, 60, "plain");
    child.send(QUESTION);
    harness::wait_for_stream(&child.stdout_buf, ANSWER.as_bytes(), "the answer");
    let observed = settle(&fixture, child, &mut server);

    let golden = record("plain_question", &observed);
    let calls: Vec<_> = golden
        .tool_calls
        .iter()
        .map(|call| (call.name.as_str(), call.outcome.as_str()))
        .collect();
    assert_eq!(
        calls,
        vec![("schema_discovery", "ok"), ("bounded_sql_query", "ok")],
        "the model discovered the schema, then queried it: {calls:?}"
    );
    assert_eq!(
        golden
            .tool_calls
            .iter()
            .find(|call| call.name == "bounded_sql_query")
            .and_then(|call| call.sql.as_deref()),
        Some("SELECT SUM(amount) AS total FROM revenue")
    );
    assert_last_state(&golden, "answer");
    assert_golden("plain_question", &golden);
}

#[test]
fn a_clarification_is_asked_then_answered_in_the_next_message() {
    let fixture = fixture("clarify");
    let mut server = harness::spawn_script_server(
        listener(),
        vec![
            ScriptStep::Clarify {
                delay: Duration::from_secs(0),
            },
            ScriptStep::Natural,
        ],
        Arc::new(AtomicBool::new(false)),
    );
    let mut child = spawn(&fixture, &server.address, 60, "clarify");
    child.send(QUESTION);
    harness::wait_for_stream(&child.stdout_buf, "⏸ question".as_bytes(), "the question");
    child.send(b"option a\n");
    harness::wait_for_stream(&child.stdout_buf, ANSWER.as_bytes(), "the answer");
    let observed = settle(&fixture, child, &mut server);

    let golden = record("clarification", &observed);
    assert_eq!(golden.tool_calls.len(), 1);
    assert_eq!(golden.tool_calls[0].name, "request_clarification");
    assert_eq!(golden.tool_calls[0].outcome, "ok");
    assert!(
        golden
            .states
            .iter()
            .any(|state| state.kind == "clarification"),
        "the user saw the question: {:?}",
        golden.states
    );
    assert_last_state(&golden, "answer");
    assert_golden("clarification", &golden);
}

#[test]
fn an_exhausted_question_budget_stops_the_turn() {
    let fixture = fixture("budget");
    let release = Arc::new(AtomicBool::new(false));
    let mut server = harness::spawn_script_server(
        listener(),
        vec![ScriptStep::HoldOpen {
            marker: String::new(),
        }],
        release.clone(),
    );
    let mut child = spawn(&fixture, &server.address, 8, "budget");
    child.send(QUESTION);
    harness::wait_for_stream_timeout(
        &child.stderr_buf,
        "the run stopped on its".as_bytes(),
        "the budget diagnostic",
        Duration::from_secs(30),
    );
    release.store(true, Ordering::SeqCst);
    let observed = settle(&fixture, child, &mut server);

    let golden = record("question_budget_stop", &observed);
    assert!(!golden.requests.is_empty(), "the held request was recorded");
    assert_last_state(&golden, "budget-stop");
    assert_golden("question_budget_stop", &golden);
}

#[test]
fn a_write_statement_is_refused_by_safety_then_answered() {
    let fixture = fixture("write");
    let mut server = harness::spawn_script_server(
        listener(),
        vec![
            ScriptStep::ToolCall {
                name: "bounded_sql_query".into(),
                arguments: serde_json::json!({"sql": "DELETE FROM revenue"}),
            },
            ScriptStep::Natural,
        ],
        Arc::new(AtomicBool::new(false)),
    );
    let mut child = spawn(&fixture, &server.address, 60, "write");
    child.send(QUESTION);
    harness::wait_for_stream(&child.stdout_buf, ANSWER.as_bytes(), "the answer");
    let observed = settle(&fixture, child, &mut server);

    let golden = record("write_refused", &observed);
    assert_eq!(golden.tool_calls.len(), 1);
    assert_eq!(golden.tool_calls[0].name, "bounded_sql_query");
    assert_eq!(
        golden.tool_calls[0].sql.as_deref(),
        Some("DELETE FROM revenue")
    );
    assert_eq!(
        golden.tool_calls[0].outcome, "refused-by-safety",
        "the safety layer refused the write"
    );
    assert_last_state(&golden, "answer");
    assert_golden("write_refused", &golden);
}
