//! The clarification ask is the database plugin's tool: a session with no
//! database offers it nowhere, and a session with one both offers it and
//! serves the call — the turn ends on the question.

use std::sync::Arc;
use std::time::Duration;

use saya_agent::{AgentEvent, ApprovalPolicy, NoopEventSink};
use serde_json::json;

use super::ChatFront;
use super::compose::RunComposition;
use super::compose_fixture::{self, Composed, RootGuard};
use super::operator_front_fixture;
use crate::commands::coding::parent_turn_provider::{Reply, Scripted};

/// Load ceiling, not a product bound: how long one scripted turn may take.
const RUN: Duration = Duration::from_secs(120);

/// The name the database plugin owns and no database-less run may offer.
const ASK: &str = "request_clarification";

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

/// Test 1: a session with no database offers no ask. The tool is the database
/// plugin's, and a plugin that never activated contributes no name — a session
/// whose work is code alone has no database question to put.
#[tokio::test]
async fn a_session_with_no_database_advertises_no_clarification_ask() {
    let provider = Scripted::start_with_script(vec![Reply::Text("hello")]);
    let fixture = compose_fixture::in_place("ask-no-db", &provider.endpoint).await;
    let front = operator_front_fixture::non_interactive();
    let (composition, _root) = compose_case(fixture, front.front(), "session-ask-no-db").await;
    let turn = tokio::time::timeout(
        RUN,
        composition.job().run_next_turn(&NoopEventSink, "hello"),
    )
    .await
    .expect("the turn settles");
    assert!(turn.is_ok(), "the turn must run: {turn:?}");

    let mut bodies = provider.bodies();
    let body = bodies.remove(0);
    assert!(
        !advertised(&body).iter().any(|name| name == ASK),
        "a session with no database offers no ask: {:?}",
        advertised(&body)
    );
}

/// Test 2: a session with a database offers the ask, and a call to it is
/// served — the turn ends on the question and no further request is sent.
#[tokio::test]
async fn a_session_with_a_database_advertises_and_serves_the_ask() {
    let provider = Scripted::start_with_script(vec![Reply::Call {
        name: ASK,
        arguments: json!({"question": "Which one?", "options": ["a", "b"]}),
    }]);
    let fixture = compose_fixture::in_place_with_database("ask-db", &provider.endpoint).await;
    let front = operator_front_fixture::non_interactive();
    let (composition, _root) = compose_case(fixture, front.front(), "session-ask-db").await;
    let turn = tokio::time::timeout(
        RUN,
        composition.job().run_next_turn(&NoopEventSink, "which one"),
    )
    .await
    .expect("the turn settles")
    .expect("the turn runs");

    let bodies = provider.bodies();
    let offered = advertised(&bodies[0]);
    assert!(
        offered.iter().any(|name| name == ASK),
        "a session with a database offers the ask: {offered:?}"
    );
    assert_eq!(
        offered.iter().filter(|name| *name == ASK).count(),
        1,
        "the plugin offers the ask once, not once per composed lane: {offered:?}"
    );
    assert_eq!(
        bodies.len(),
        1,
        "a served ask ends the turn: no further request"
    );
    assert!(
        turn.events.iter().any(|event| matches!(
            event,
            AgentEvent::ClarificationNeeded { question, .. } if question == "Which one?"
        )),
        "the served call put the question to the user: {:?}",
        turn.events
    );
}
