//! What the chat surface advertises with no evidence in the request and no
//! `[tools] allow` list: the always-on list in `perf_guard`, nothing else.
//!
//! A request that states nothing is the common case — most messages are a
//! question about the folder — so the list that answers it is the one that
//! decides what every turn costs. A name outside the list must be gated behind
//! evidence, or added to the list in review.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::sync::Arc;
use std::time::Duration;

use saya_agent::{ApprovalPolicy, NoopEventSink};

use super::ChatFront;
use super::compose::RunComposition;
use super::compose_fixture::{self, Composed, RootGuard};
use super::operator_front_fixture;
use crate::commands::coding::parent_turn_provider::{Reply, Scripted};
use crate::perf_guard;

/// Load ceiling, not a product bound: how long one scripted turn may take.
const TURN: Duration = Duration::from_secs(120);

/// The message: it names no URL, no helper and no database word, so every
/// optional tool stays off.
const PLAIN: &str = "summarise the notes in this folder";

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

/// The `(name, description)` pairs one request advertises.
fn advertised(body: &str) -> Vec<(String, String)> {
    let body: serde_json::Value = serde_json::from_str(body).expect("a request body is JSON");
    body["tools"]
        .as_array()
        .expect("a request advertises its tools")
        .iter()
        .filter_map(|tool| {
            let name = tool["function"]["name"].as_str()?.to_owned();
            let description = tool["function"]["description"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            Some((name, description))
        })
        .collect()
}

/// Test 1 — a message that states nothing is answered by the always-on list
/// and by nothing else.
#[tokio::test]
async fn a_plain_message_advertises_only_the_always_on_list() {
    let provider = Scripted::start_with_script(vec![Reply::Text("done")]);
    let fixture = compose_fixture::in_place_with_database("guard-chat", &provider.endpoint).await;
    let front = operator_front_fixture::non_interactive();
    let (composition, _root) = compose_case(fixture, front.front(), "guard-chat").await;
    let turn = tokio::time::timeout(TURN, composition.job().run_next_turn(&NoopEventSink, PLAIN))
        .await
        .expect("the turn settles");
    assert!(turn.is_ok(), "the turn must run: {turn:?}");

    let bodies = provider.bodies();
    let advertised = advertised(&bodies[0]);
    let names: Vec<&str> = advertised.iter().map(|(name, _)| name.as_str()).collect();
    println!("chat default advertisement: {names:?}");
    perf_guard::assert_only_listed("chat", perf_guard::ALWAYS_ON_CHAT, &names);
}

/// Test 2 — every tool the chat advertises carries a description within the
/// cap: a turn pays for each one on every request.
#[tokio::test]
async fn the_chats_descriptions_fit_the_cap() {
    let provider = Scripted::start_with_script(vec![Reply::Text("done")]);
    let fixture =
        compose_fixture::in_place_with_database("guard-chat-cap", &provider.endpoint).await;
    let front = operator_front_fixture::non_interactive();
    let (composition, _root) = compose_case(fixture, front.front(), "guard-chat-cap").await;
    let turn = tokio::time::timeout(TURN, composition.job().run_next_turn(&NoopEventSink, PLAIN))
        .await
        .expect("the turn settles");
    assert!(turn.is_ok(), "the turn must run: {turn:?}");

    let bodies = provider.bodies();
    let advertised = advertised(&bodies[0]);
    let longest = advertised
        .iter()
        .map(|(name, description)| (description.len(), name.clone()))
        .max();
    println!("chat longest description: {longest:?}");
    perf_guard::assert_descriptions_within("chat", &advertised);
}
