//! The part-B database-golden scenarios: recall injected into the first
//! request, post-turn learning written after an answer, a contract-tool call,
//! and a saved-investigation replay (which never touches a provider).
//!
//! Each asserts the behaviour it exists to pin before comparing the golden,
//! so an update run cannot quietly bless a scenario that never ran.

use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use super::harness::{self, ScriptStep};
use super::record::{Golden, assert_golden};
use super::setup::{ANSWER, QUESTION, assert_last_state, fixture, settle, visible_states};
use super::setup_b::{SEEDED_CLAIM, seed_claim, spawn_assisted, stored_claims, wait_for_requests};

/// The user's message for the learning row: it asserts a fact, so the
/// extractor's `user_explicit` proposal is corroborated and lands confirmed.
const LEARNING_QUESTION: &[u8] = b"remember the revenue total is recorded\n";

/// One scripted extractor reply: one confirmed table description for `T0`.
const EXTRACTION_REPLY: &str = concat!(
    r#"{"proposals": [{"object_id": "T0", "slot": "table.description", "#,
    r#""value": "one row per revenue entry", "origin": "user_explicit"}]}"#
);

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

fn proposal_count(reply: &str) -> usize {
    serde_json::from_str::<serde_json::Value>(reply).unwrap()["proposals"]
        .as_array()
        .map_or(0, Vec::len)
}

#[test]
fn a_seeded_claim_is_recalled_into_the_first_request() {
    let fixture = fixture("recall");
    seed_claim(&fixture, "recall", SEEDED_CLAIM);
    let mut server = harness::spawn_script_server(
        listener(),
        vec![ScriptStep::Natural, ScriptStep::Harmless],
        Arc::new(AtomicBool::new(false)),
    );
    let mut child = spawn_assisted(&fixture, &server.address, 60, "recall");
    child.send(QUESTION);
    harness::wait_for_stream(&child.stdout_buf, ANSWER.as_bytes(), "the answer");
    // The recalled claim makes the turn proposal-worthy, so extraction runs
    // after the answer; wait for it rather than racing the exit.
    wait_for_requests(&server, 2);
    let observed = settle(&fixture, child, &mut server);

    let golden = record("recall_injected", &observed);
    let first = &golden.requests[0];
    let recall = first
        .recall
        .as_ref()
        .expect("the first request carries the recall block");
    assert!(recall.present);
    assert!(recall.body_bytes > 0, "the block body is non-empty");
    assert_last_state(&golden, "answer");
    assert_golden("recall_injected", &golden);
}

#[test]
fn an_answered_turn_is_followed_by_extraction_that_stores_a_claim() {
    let fixture = fixture("learning");
    let mut server = harness::spawn_script_server(
        listener(),
        vec![
            ScriptStep::ToolCall {
                name: "bounded_sql_query".into(),
                arguments: serde_json::json!({
                    "sql": "SELECT SUM(amount) AS total FROM revenue"
                }),
            },
            ScriptStep::Natural,
            ScriptStep::Json(EXTRACTION_REPLY.into()),
        ],
        Arc::new(AtomicBool::new(false)),
    );
    let mut child = spawn_assisted(&fixture, &server.address, 60, "learning");
    child.send(LEARNING_QUESTION);
    harness::wait_for_stream(&child.stdout_buf, ANSWER.as_bytes(), "the answer");
    wait_for_requests(&server, 3);
    harness::wait_for_stream(
        &child.stdout_buf,
        "memory learned".as_bytes(),
        "the learned line",
    );
    let observed = settle(&fixture, child, &mut server);

    let golden = record("learning_written", &observed).with_learning_counts(
        proposal_count(EXTRACTION_REPLY),
        stored_claims(&observed.stdout),
    );
    let learning = golden.learning.as_ref().expect("extraction was recorded");
    assert_eq!(learning.extraction_requests, 1, "one extraction request");
    assert_eq!(learning.proposed, 1, "the scripted reply offered one claim");
    assert_eq!(learning.stored, 1, "the user saw one claim stored");
    assert_eq!(golden.tool_calls.len(), 1);
    assert_eq!(golden.tool_calls[0].name, "bounded_sql_query");
    assert_eq!(golden.tool_calls[0].outcome, "ok");
    assert_last_state(&golden, "answer");
    assert_golden("learning_written", &golden);
}

#[test]
fn the_model_can_search_contracts() {
    let fixture = fixture("contract");
    let mut server = harness::spawn_script_server(
        listener(),
        vec![
            ScriptStep::ToolCall {
                name: "contract_search".into(),
                arguments: serde_json::json!({"terms": ["revenue"]}),
            },
            ScriptStep::Natural,
        ],
        Arc::new(AtomicBool::new(false)),
    );
    // Memory off: the contract tools are still advertised because a state
    // store exists and data sharing is on (the part-A surface).
    let mut child = super::setup::spawn(&fixture, &server.address, 60, "contract");
    child.send(QUESTION);
    harness::wait_for_stream(&child.stdout_buf, ANSWER.as_bytes(), "the answer");
    let observed = settle(&fixture, child, &mut server);

    let golden = record("contract_search", &observed);
    assert_eq!(golden.tool_calls.len(), 1);
    assert_eq!(golden.tool_calls[0].name, "contract_search");
    assert_eq!(golden.tool_calls[0].outcome, "ok");
    assert_last_state(&golden, "answer");
    assert_golden("contract_search", &golden);
}

// The replay row has no model loop and lives in its own module so this file
// stays focused on provider-driven scenarios.
#[path = "scenarios_b_investigation.rs"]
mod investigation;
