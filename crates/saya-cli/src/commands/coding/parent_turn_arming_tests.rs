//! A plugin's turn policy, armed on the turn's first use of that plugin (C43).
//!
//! A core turn holds one deferred budget, unarmed: a turn that never uses the
//! plugin spends none of the plugin's policy. It arms on the first of (a) a
//! tool call the registry routes to the plugin — before the call executes — and
//! (b) the plugin's `context()` contributing a block to this turn's prompt,
//! which happens before request 1, so the first request counts.
//!
//! The stop is the turn's: the run keeps taking turns. The elapsed dimension,
//! whose sleep starts at the arm, is `parent_turn_arming_elapsed_tests`.

#![cfg(unix)]

use super::parent_arming_fixture::{
    CONTEXT_BODY, RUN, TOOL, TestPlugin, first_turn, inputs, one_request,
};
use super::parent_turn_fixture::in_place_writing;
use super::parent_turn_provider::{Reply, Scripted};
use saya_agent::NoopEventSink;
use serde_json::json;
use std::sync::Arc;

/// Test 1 — a turn that never uses the plugin makes three requests and answers
/// normally: the plugin's one-request policy is never armed.
#[tokio::test]
async fn a_turn_that_never_uses_the_plugin_is_not_budgeted() {
    let provider = Scripted::start_with_script(vec![
        Reply::DistinctWrite,
        Reply::DistinctWrite,
        Reply::Text("the unarmed turn answered"),
    ]);
    let fixture = in_place_writing("arming-unused", &provider.endpoint).await;
    let plugin = Arc::new(TestPlugin::new(one_request(), false));

    let output = first_turn(&fixture.job, inputs(Arc::clone(&plugin)))
        .await
        .expect("a turn that never uses the plugin answers normally");
    assert_eq!(output.answer, "the unarmed turn answered");
    assert!(
        !output.truncated,
        "no plugin budget stopped the turn: its answer is whole"
    );
    assert_eq!(
        provider.bodies().len(),
        3,
        "three requests were served: none of them was budgeted"
    );
    assert_eq!(
        fixture.job.turn_stop(),
        None,
        "nothing stopped the turn, so there is no budget wording"
    );
    assert_eq!(plugin.executed(), 0, "the plugin's tool never ran");
}

/// Test 2 — the turn's first routed call arms the plugin's budget, and the
/// request after that call is refused: the run stays open.
#[tokio::test]
async fn the_request_after_a_routed_call_is_refused_and_the_run_stays_open() {
    let provider = Scripted::start_with_script(vec![
        Reply::Call {
            name: TOOL,
            arguments: json!({"text": "hello"}),
        },
        Reply::Text("the next turn answered"),
    ]);
    let fixture = in_place_writing("arming-routed", &provider.endpoint).await;
    let plugin = Arc::new(TestPlugin::new(one_request(), false));

    let stopped = first_turn(&fixture.job, inputs(Arc::clone(&plugin)))
        .await
        .expect("a spent budget is a partial answer, not a run failure");
    assert_eq!(plugin.executed(), 1, "the call was routed to the plugin");
    assert_eq!(
        provider.bodies().len(),
        1,
        "the request after the routed call was refused"
    );
    let stop = fixture
        .job
        .turn_stop()
        .expect("the turn states what stopped it");
    assert_eq!(
        stop.message(),
        "the run stopped on its logical-requests budget; any answer it produced is partial or absent",
        "the plugin's own request ceiling stopped the turn"
    );
    assert!(stopped.truncated, "the answer it produced is partial");
    assert!(
        fixture.job._in_place_lease.is_some(),
        "the run is still open and holds its one folder lease"
    );

    let next = tokio::time::timeout(RUN, fixture.job.run_next_turn(&NoopEventSink, "again"))
        .await
        .expect("the further turn must settle");
    assert!(next.is_ok(), "a spent budget is per turn: {next:?}");
    assert_eq!(
        provider.bodies().len(),
        2,
        "the further turn reached the provider"
    );
}

/// Test 3 — a plugin whose context reached this turn's prompt arms before
/// request 1, so the first request is the one its budget counts.
#[tokio::test]
async fn plugin_context_arms_the_budget_before_the_first_request() {
    let provider = Scripted::start_with_script(vec![
        Reply::DistinctWrite,
        Reply::Text("the second request would answer"),
    ]);
    let fixture = in_place_writing("arming-context", &provider.endpoint).await;
    let plugin = Arc::new(TestPlugin::new(one_request(), true));

    first_turn(&fixture.job, inputs(plugin))
        .await
        .expect("a spent budget is a partial answer");
    assert_eq!(
        provider.bodies().len(),
        1,
        "the turn armed before request 1, so the second request was refused"
    );
    assert!(
        provider.bodies()[0].contains(CONTEXT_BODY),
        "the block that armed the budget rode the request it armed for"
    );
    let stop = fixture
        .job
        .turn_stop()
        .expect("the turn states what stopped it");
    assert_eq!(
        stop.message(),
        "the run stopped on its logical-requests budget; any answer it produced is partial or absent"
    );
}
