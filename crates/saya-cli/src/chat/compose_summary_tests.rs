//! The launch summary rides the first request, so a greeting costs no call.
//!
//! The scan itself is bounded and tested beside it
//! (`chat::workspace_summary_tests`); this is that summary on the live path: a
//! composed chat session opens its run over a folder, the summary is gathered
//! in the background, and the run's FIRST turn carries it — on the untrusted
//! lane the project instructions ride, after them — so a first message about
//! the folder is answered from one request and no tool call.

#![cfg(unix)]

use std::fs;
use std::sync::Arc;
use std::time::{Duration, Instant};

use saya_agent::{ApprovalPolicy, NoopEventSink};

use super::compose::RunComposition;
use super::compose_fixture::{self, RootGuard};
use super::operator_front_fixture;
use crate::commands::coding::NativeCodingJob;
use crate::commands::coding::parent_turn_provider::{Reply, Scripted};

/// Load ceiling, not a product bound: how long one scripted turn may take.
const RUN: Duration = Duration::from_secs(120);

/// Load ceiling for the launch scan: how long a test waits for it to land.
const SCAN: Duration = Duration::from_secs(20);

/// The label the summary reaches the model under.
const SUMMARY_LABEL: &str = "source: workspace-summary";

/// A folder with something to state: a directory, a manifest, the project's
/// instructions, and one file a first listing names.
fn populate(root: &RootGuard) {
    let folder = root.checkout_root();
    fs::create_dir(folder.join("src")).expect("the fixture directory creates");
    fs::write(folder.join("Cargo.toml"), "[package]\n").expect("the fixture manifest writes");
    fs::write(folder.join("AGENTS.md"), "guard rails\n").expect("the fixture guidance writes");
}

/// The composed run over a folder the test populated, its provider, and the
/// fixture root that must outlive it.
async fn composed(case: &str, replies: Vec<Reply>) -> (Scripted, Arc<NativeCodingJob>, RootGuard) {
    let provider = Scripted::start_with_script(replies);
    let fixture = compose_fixture::in_place(case, &provider.endpoint).await;
    let front = operator_front_fixture::non_interactive();
    let composed = fixture.into_parts(front.front(), case, ApprovalPolicy::Ask);
    populate(&composed.root);
    let composition = RunComposition::compose(composed.application, composed.run)
        .await
        .expect("the session's run composes");
    composition.open().await.expect("the run opens");
    (provider, Arc::clone(composition.job()), composed.root)
}

/// Bounded wait for the launch scan: the message a person sends a moment after
/// the prompt paints is the one the summary rides, so a test waits for the same
/// moment rather than for a fixed nap.
fn wait_for_scan(job: &NativeCodingJob) {
    let deadline = Instant::now() + SCAN;
    while !job.workspace_summary_ready() {
        assert!(
            Instant::now() < deadline,
            "the launch scan never finished within {SCAN:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The messages one provider request carried, and the text of the user turn.
fn user_turn(body: &str) -> String {
    let body: serde_json::Value = serde_json::from_str(body).expect("the request is JSON");
    let messages = body["messages"]
        .as_array()
        .expect("the request states its messages");
    let user = messages.last().expect("the turn's own prompt is last");
    user["content"].as_str().unwrap_or_default().to_owned()
}

/// "Hi" is answered by the first request and no tool call: the folder the
/// question is about is already in front of the model.
#[tokio::test]
async fn a_greeting_is_answered_without_a_tool_call_once_the_summary_is_ready() {
    let (provider, job, _root) = composed("summary-hi", vec![Reply::Text("Hi!")]).await;
    wait_for_scan(&job);
    let turn = tokio::time::timeout(RUN, job.run_next_turn(&NoopEventSink, "Hi"))
        .await
        .expect("the turn settles");
    assert!(turn.is_ok(), "the turn must run: {turn:?}");
    let bodies = provider.bodies();
    assert_eq!(
        bodies.len(),
        1,
        "the greeting was answered by one request and no tool call: {}",
        bodies.len()
    );
    let lane = user_turn(&bodies[0]);
    assert!(
        lane.contains(SUMMARY_LABEL),
        "the first request carries the workspace summary: {lane}"
    );
    assert!(
        lane.contains("Cargo.toml") && lane.contains("src/") && lane.contains("(Rust)"),
        "the summary states the folder's own shape: {lane}"
    );
    assert!(
        lane.contains("AGENTS.md"),
        "the summary names the instruction sources: {lane}"
    );
}

/// The summary is the run's launch fact: the turn after the first carries none
/// of it, and the request the model sees then is the conversation's own.
#[tokio::test]
async fn the_second_turn_does_not_carry_the_summary_again() {
    let (provider, job, _root) = composed("summary-second", Vec::new()).await;
    wait_for_scan(&job);
    let first = tokio::time::timeout(RUN, job.run_next_turn(&NoopEventSink, "what is here?"))
        .await
        .expect("the first turn settles");
    assert!(first.is_ok(), "the first turn must run: {first:?}");
    let second = tokio::time::timeout(RUN, job.run_next_turn(&NoopEventSink, "and then?"))
        .await
        .expect("the second turn settles");
    assert!(second.is_ok(), "the second turn must run: {second:?}");
    let bodies = provider.bodies();
    assert_eq!(bodies.len(), 2, "the run sent a request per turn");
    assert!(
        user_turn(&bodies[0]).contains(SUMMARY_LABEL),
        "the first turn carries the summary: {}",
        user_turn(&bodies[0])
    );
    assert!(
        !user_turn(&bodies[1]).contains(SUMMARY_LABEL),
        "the second turn pays nothing for a folder already stated: {}",
        user_turn(&bodies[1])
    );
}
