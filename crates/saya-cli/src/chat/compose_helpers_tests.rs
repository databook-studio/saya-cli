//! The session advertises the job root's own helper tool, under the root's
//! rule: the optional tools follow what the run was asked.
//!
//! A chat session's request carries the session's name and never the
//! operator's message, so the rule is stated where the run actually asks —
//! its first turn — rather than by a second list of names a chat may offer.
//! A message that names a helper gets the tool; one that does not, does not.

use std::sync::Arc;
use std::time::Duration;

use saya_agent::{ApprovalPolicy, NoopEventSink};

use super::ChatFront;
use super::compose::RunComposition;
use super::compose_fixture::{self, Composed, RootGuard};
use super::operator_front_fixture;
use crate::commands::coding::parent_turn_provider::{Reply, Scripted};

/// Load ceiling, not a product bound: how long one scripted turn may take.
const RUN: Duration = Duration::from_secs(120);

/// The composed and opened run over one fixture, with the folder kept alive.
async fn compose_case(
    fixture: compose_fixture::Fixture,
    front: Arc<dyn ChatFront>,
    session: &'static str,
) -> (RunComposition, RootGuard) {
    let Composed {
        application,
        run,
        root,
    } = fixture.into_parts(front, session, ApprovalPolicy::Ask);
    let composition = RunComposition::compose(application, run)
        .await
        .expect("the session's run composes");
    composition.open().await.expect("the run opens");
    (composition, root)
}

/// The tool names one request advertises, in order.
fn advertised(body: &str) -> Vec<String> {
    let body: serde_json::Value = serde_json::from_str(body).expect("a request body is JSON");
    body["tools"]
        .as_array()
        .expect("a request advertises its tools")
        .iter()
        .filter_map(|tool| tool["function"]["name"].as_str().map(str::to_owned))
        .collect()
}

/// One turn on the composed run, carrying the operator's own words.
async fn turn(composition: &RunComposition, message: &str) {
    let turn = tokio::time::timeout(
        RUN,
        composition.job().run_next_turn(&NoopEventSink, message),
    )
    .await
    .expect("the turn settles");
    assert!(turn.is_ok(), "the turn must run: {turn:?}");
}

/// A session whose first message asks for helpers is offered the tool the job
/// root is offered for the same words.
#[tokio::test]
async fn a_first_message_that_asks_for_a_helper_advertises_it() {
    let provider = Scripted::start_with_script(vec![Reply::Text("hello")]);
    let fixture = compose_fixture::in_place("helpers-advertise", &provider.endpoint).await;
    let front = operator_front_fixture::non_interactive();
    let (composition, _root) = compose_case(fixture, front.front(), "session-helpers").await;
    turn(
        &composition,
        "delegate this: use two helpers to find where pricing is tested",
    )
    .await;

    let bodies = provider.bodies();
    let offered = advertised(&bodies[0]);
    assert!(
        offered.iter().any(|name| name == "helpers"),
        "a message that asks for helpers is offered the helper tool: {offered:?}"
    );
}

/// The same rule from the other side: a message that names no helper is offered
/// none, exactly as a job root whose task names none is.
#[tokio::test]
async fn a_message_that_names_no_helper_advertises_none() {
    let provider = Scripted::start_with_script(vec![Reply::Text("hello")]);
    let fixture = compose_fixture::in_place("helpers-withheld", &provider.endpoint).await;
    let front = operator_front_fixture::non_interactive();
    let (composition, _root) = compose_case(fixture, front.front(), "session-no-helpers").await;
    turn(&composition, "what is this folder for?").await;

    let bodies = provider.bodies();
    let offered = advertised(&bodies[0]);
    assert!(
        !offered.iter().any(|name| name == "helpers"),
        "a message that names no helper is offered none: {offered:?}"
    );
}
