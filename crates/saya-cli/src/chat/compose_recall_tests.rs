//! What a session's memory mode decides about the first request it sends: with
//! memory on and a profile active, turn 1 carries the database-contracts block
//! recalled from the session's store; with memory off it carries none.
//!
//! Both rows seed the same confirmed claim into the store the session opens, so
//! the difference the block shows is the memory mode and nothing else — the
//! store has something to recall either way.

use std::time::Duration;

use saya_agent::{ApprovalPolicy, NoopEventSink};
use saya_config::MemoryMode;

use super::compose::RunComposition;
use super::compose_fixture::{self, Composed, RootGuard};
use super::compose_recall_fixture::{self, QUESTION, RECALL_SOURCE, identity_of};
use super::operator_front_fixture;
use crate::commands::coding::parent_turn_provider::{Reply, Scripted};

/// Load ceiling, not a product bound: how long one scripted turn may take.
const RUN: Duration = Duration::from_secs(120);

/// The composed run over one fixture, with the folder kept alive.
struct Case {
    composition: RunComposition,
    _root: RootGuard,
}

/// Seeds the store, states the session's memory facts, and composes and opens
/// the run over them — asking nobody.
async fn compose_case(
    fixture: compose_fixture::Fixture,
    session: &'static str,
    memory: MemoryMode,
) -> Case {
    let front = operator_front_fixture::non_interactive();
    let Composed {
        application,
        mut run,
        root,
    } = fixture.into_parts(front.front(), session, ApprovalPolicy::Ask);
    run.memory = memory;
    run.memory_reachable = memory != MemoryMode::Off;
    let identity = identity_of(&run.connections);
    compose_recall_fixture::seed::seed_confirmed_claim(application.store(), &identity).await;
    let composition = RunComposition::compose(application, run)
        .await
        .expect("the session's run composes");
    composition.open().await.expect("the run opens");
    Case {
        composition,
        _root: root,
    }
}

/// The run's first turn, waited for under the fixture's load ceiling, and the
/// first request body it sent.
async fn first_request(composition: &RunComposition, provider: &Scripted) -> String {
    let turn = tokio::time::timeout(
        RUN,
        composition.job().run_next_turn(&NoopEventSink, QUESTION),
    )
    .await
    .expect("the turn settles");
    assert!(turn.is_ok(), "the turn must run: {turn:?}");
    let mut bodies = provider.bodies();
    bodies.remove(0)
}

/// Memory on: the turn's first request carries the domain the store holds, so
/// the model is told what was already recorded about `orders`.
#[tokio::test]
async fn a_session_with_memory_on_recalls_its_store_into_the_first_request() {
    let provider = Scripted::start_with_script(vec![Reply::Text("hello")]);
    let fixture = compose_fixture::in_place_with_memory(
        "recall-on",
        &provider.endpoint,
        MemoryMode::Assisted,
    )
    .await;
    let case = compose_case(fixture, "session-recall-on", MemoryMode::Assisted).await;

    let body = first_request(&case.composition, &provider).await;
    assert!(
        body.contains(RECALL_SOURCE),
        "the first request carries the recalled database contracts: {body}"
    );
    assert!(
        body.contains("orders"),
        "the seeded claim's table reached the prompt: {body}"
    );
}

/// Memory off: the same seeded store, and the turn's first request carries no
/// database contracts at all — off is off.
#[tokio::test]
async fn a_session_with_memory_off_recalls_nothing_into_the_first_request() {
    let provider = Scripted::start_with_script(vec![Reply::Text("hello")]);
    let fixture =
        compose_fixture::in_place_with_memory("recall-off", &provider.endpoint, MemoryMode::Off)
            .await;
    let case = compose_case(fixture, "session-recall-off", MemoryMode::Off).await;

    let body = first_request(&case.composition, &provider).await;
    assert!(
        !body.contains(RECALL_SOURCE),
        "a memory-off run recalls nothing: {body}"
    );
}
