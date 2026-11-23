//! A session's run, composed: the database plugin, the mode cell, the operator,
//! and the open that asks nobody.
//!
//! Each test drives one thing the session states about its run and reads it back
//! from what the run actually sends and does — the request body the provider
//! received, the cell the run's calls resolve, the front the run's questions
//! reach — so nothing here asserts on a composition's internals.

use std::sync::Arc;
use std::time::Duration;

use saya_agent::{ApprovalPolicy, NoopEventSink};
use serde_json::json;

use super::ChatFront;
use super::compose::RunComposition;
use super::compose_fixture::{self, Composed, RootGuard};
use super::operator_front_fixture;
use crate::agent::tools::consent::{Answer, Caller, Operator};
use crate::commands::coding::parent_turn_provider::{Reply, Scripted};

/// Load ceiling, not a product bound: how long one scripted turn may take.
const RUN: Duration = Duration::from_secs(120);

/// The composed run over one fixture, with the folder kept alive.
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
    (composition, root)
}

/// The run's first turn, waited for under the fixture's load ceiling.
async fn first_turn(composition: &RunComposition) {
    first_turn_asking(composition, "hello").await;
}

/// The run's first turn on a request that shows database work: the database
/// family is advertised from the turn whose own words asked for it
/// (`plugins::database::family`), never from the bound profile alone.
async fn first_turn_about_data(composition: &RunComposition) {
    first_turn_asking(composition, "which tables hold the users?").await;
}

async fn first_turn_asking(composition: &RunComposition, request: &str) {
    let turn = tokio::time::timeout(
        RUN,
        composition.job().run_next_turn(&NoopEventSink, request),
    )
    .await
    .expect("the turn settles");
    assert!(turn.is_ok(), "the turn must run: {turn:?}");
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

/// The system message of one request body, as the model receives it.
fn system_message(body: &str) -> String {
    let body: serde_json::Value = serde_json::from_str(body).expect("a request body is JSON");
    body["messages"]
        .as_array()
        .expect("a request carries its messages")
        .iter()
        .filter(|message| message["role"] == "system")
        .filter_map(|message| message["content"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Test 1, with a database: the run states the database section of its prompt
/// and offers the SQL tools — the plugin is active on this run.
#[tokio::test]
async fn a_session_with_a_database_composes_the_plugin_and_its_section() {
    let provider = Scripted::start_with_script(vec![Reply::Text("hello")]);
    let fixture = compose_fixture::in_place_with_database("with-db", &provider.endpoint).await;
    let front = operator_front_fixture::non_interactive();
    let (composition, _root) = compose_case(fixture, front.front(), "session-with-db").await;
    composition.open().await.expect("the run opens");
    first_turn_about_data(&composition).await;

    let mut bodies = provider.bodies();
    let body = bodies.remove(0);
    assert!(
        advertised(&body)
            .iter()
            .any(|name| name == "bounded_sql_query"),
        "the plugin's SQL tools are advertised: {:?}",
        advertised(&body)
    );
    let system = system_message(&body);
    assert!(
        system.contains("Answer the question exactly as asked"),
        "the plugin's answer contract rode the run's prompt: {system}"
    );
    assert!(
        system.contains("Database `primary` (sqlite)"),
        "the session's database is stated as a fact: {system}"
    );
    assert!(
        system.contains("The SQL dialect is sqlite's"),
        "the dialect statement rode the run's prompt: {system}"
    );
}

/// Test 1, without a database: the plugin activates nothing, so the run sends
/// zero database bytes and offers no SQL tool.
#[tokio::test]
async fn a_session_with_no_database_sends_no_database_bytes() {
    let provider = Scripted::start_with_script(vec![Reply::Text("hello")]);
    let fixture = compose_fixture::in_place("no-db", &provider.endpoint).await;
    let front = operator_front_fixture::non_interactive();
    let (composition, _root) = compose_case(fixture, front.front(), "session-no-db").await;
    composition.open().await.expect("the run opens");
    first_turn(&composition).await;

    let mut bodies = provider.bodies();
    let body = bodies.remove(0);
    assert!(
        !advertised(&body)
            .iter()
            .any(|name| name == "bounded_sql_query"),
        "no database means no SQL tool: {:?}",
        advertised(&body)
    );
    for word in ["Database", "dialect", "schema"] {
        assert!(
            !body.contains(word),
            "an inactive plugin contributes zero database bytes, and `{word}` is in the request: {body}"
        );
    }
}

/// Test 2: the run is an attached composition, so its mode cell can still be
/// switched — and the switch is the one the run's own readers see.
#[tokio::test]
async fn the_composed_mode_cell_is_switchable() {
    let provider = Scripted::start_with_script(vec![Reply::Text("hello")]);
    let fixture = compose_fixture::in_place("mode-cell", &provider.endpoint).await;
    let front = operator_front_fixture::non_interactive();
    let (composition, _root) = compose_case(fixture, front.front(), "session-mode").await;

    let cell = composition.mode_cell();
    assert!(
        !cell.is_frozen(),
        "an attached launch leaves the session's mode switchable"
    );
    cell.set(ApprovalPolicy::ReadOnly)
        .expect("the session switches its mode");
    assert_eq!(cell.get(), ApprovalPolicy::ReadOnly);
    assert_eq!(
        composition.job().approval_mode().get(),
        ApprovalPolicy::ReadOnly,
        "every reader of the run's cell sees the switch"
    );
}

/// Test 3: the operator attached to the run is the session's own — the question
/// it puts reaches the session's front, and the grant a session answer records
/// lands in the store [`RunComposition::operator`] hands out.
#[tokio::test]
async fn the_operator_attached_is_the_sessions_own() {
    let provider = Scripted::start_with_script(vec![Reply::Text("hello")]);
    let fixture = compose_fixture::in_place("operator", &provider.endpoint).await;
    let mut front = operator_front_fixture::interactive();
    let (composition, _root) = compose_case(fixture, front.front(), "session-operator").await;

    let attached = composition
        .job()
        .operator()
        .expect("the session's operator is attached to the run");
    // The line the root's own tool puts a call under: it names the run that
    // asks, which is the run the operator was attached for.
    let subject = Caller::Root {
        job_id: composition.job().job_id().clone(),
    }
    .line("Run a shell command", "run a shell command");
    let ask =
        tokio::spawn(async move { attached.answer(&subject, &json!({"command": "ls"})).await });
    let pending = tokio::time::timeout(RUN, front.next())
        .await
        .expect("the run's operator put its question to the session's front");
    assert!(
        pending.question.caller.contains("session-operator"),
        "the question names the run that asks: {}",
        pending.question.caller
    );
    let grant = pending
        .question
        .grant
        .clone()
        .expect("a shell call's question names the grant a session answer records");
    pending.answer(Answer::Session);
    assert_eq!(ask.await.expect("the ask settles"), Answer::Session);
    let grants = composition
        .operator()
        .grants()
        .expect("the session's operator keeps a run store");
    assert!(
        grants.is_granted(&grant),
        "the attached operator recorded the answer in the session's own store"
    );
}

/// Test 4: opening a run asks nobody — consent to open is the session's mode —
/// and the run it leaves open takes the session's first message as its turn.
#[tokio::test]
async fn opening_a_run_asks_nobody() {
    let provider = Scripted::start_with_script(vec![Reply::Text("hello")]);
    let fixture = compose_fixture::in_place("quiet-open", &provider.endpoint).await;
    let front = operator_front_fixture::interactive();
    let (composition, _root) = compose_case(fixture, front.front(), "session-open").await;

    tokio::time::timeout(RUN, composition.open())
        .await
        .expect("the open asks nobody")
        .expect("the run opens");
    assert_eq!(
        front.seen(),
        0,
        "opening a run puts no question to the front"
    );
    first_turn(&composition).await;
}
