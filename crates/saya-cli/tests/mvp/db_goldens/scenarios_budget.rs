//! The re-specified budget row (`ref/05` §3): the stop a database-armed turn
//! makes, and the turn after it.
//!
//! It lives in its own file because it is the one scenario this job re-states;
//! part A's other scenarios are the behaviour the refactor must keep.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::harness::{self, ScriptStep};
use super::record::{StateRecord, assert_golden};
use super::scenarios::{listener, record_states};
use super::setup::{ANSWER, QUESTION, assert_last_state, fixture, settle, spawn, visible_states};

/// The sentence a database-budget stop says, verbatim: the one wording the
/// budget and the REPL's own diagnostic share (`budget_wording`).
const BUDGET_STOP: &str =
    "the run stopped on its elapsed-time budget; any answer it produced is partial or absent";

/// The second line of the budget row: the reply the run takes after the stop,
/// which proves the stop ended the turn and not the run.
const NEXT_LINE: &[u8] = b"what about last month\n";

#[test]
fn an_exhausted_question_budget_stops_the_turn() {
    let fixture = fixture("budget");
    let release = Arc::new(AtomicBool::new(false));
    let mut server = harness::spawn_script_server(
        listener(),
        vec![
            // Turn 1: the model's own database call. It is the turn's first
            // database use, and that is what arms `[limits.database]` — the
            // budget a turn never uses spends nothing.
            ScriptStep::ToolCall {
                name: "schema_discovery".into(),
                arguments: serde_json::json!({}),
            },
            // The request carrying that call's result is held open and silent,
            // so the armed 8s elapsed ceiling is what ends turn 1.
            ScriptStep::HoldOpen {
                marker: String::new(),
            },
            // Turn 2: the line after the stop is a turn of the same run.
            ScriptStep::Natural,
        ],
        release.clone(),
    );
    let mut child = spawn(&fixture, &server.address, 8, "budget");
    child.send(QUESTION);
    harness::wait_for_stream_timeout(
        &child.stderr_buf,
        "the run stopped on its".as_bytes(),
        "the budget diagnostic",
        Duration::from_secs(60),
    );
    // The held response is released so the mock can serve the run's next turn.
    release.store(true, Ordering::SeqCst);
    child.send(NEXT_LINE);
    harness::wait_for_stream(&child.stdout_buf, ANSWER.as_bytes(), "the answer");
    let observed = settle(&fixture, child, &mut server);

    // The two turns settled in this order: the budget stop ended turn 1, and
    // the answer ended turn 2. `visible_states` reads stdout before stderr, so
    // the order the user saw is stated here.
    let mut states = visible_states(&observed.stdout, &observed.stderr);
    states.sort_by_key(|state| u8::from(state.kind != "budget-stop"));
    let golden = record_states("question_budget_stop", &observed, states);

    let calls: Vec<_> = golden
        .tool_calls
        .iter()
        .map(|call| (call.name.as_str(), call.outcome.as_str()))
        .collect();
    assert_eq!(
        calls,
        vec![("schema_discovery", "ok")],
        "the one database call the model made armed the budget: {calls:?}"
    );
    assert_eq!(
        golden.requests.len(),
        3,
        "turn 1's two requests (the call, then the held one) and turn 2's: {:?}",
        golden
            .requests
            .iter()
            .map(|r| r.messages)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        golden.states,
        vec![
            StateRecord {
                kind: "budget-stop".into(),
                text: BUDGET_STOP.into(),
            },
            StateRecord {
                kind: "answer".into(),
                text: ANSWER.into(),
            },
        ],
        "turn 1 stopped on the database budget, turn 2 answered: {:?}",
        golden.states
    );
    // The budget is said on stderr, once: a second line would word one stop
    // twice, and a line on stdout would be part of the answer.
    assert!(
        observed.stderr.matches("the run stopped on its").count() == 1,
        "the budget line appears once on stderr: {:?}",
        observed.stderr
    );
    assert!(
        !observed.stdout.contains("the run stopped on its"),
        "the budget line is a diagnostic, never the answer: {:?}",
        observed.stdout
    );
    assert_last_state(&golden, "answer");
    assert_golden("question_budget_stop", &golden);
}
