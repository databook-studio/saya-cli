//! The surface a session's mode advertises: a run whose mode cannot reach a
//! write-shaped call offers none of them, and the rule is read per request, so a
//! mode switched on the run changes the list the next turn sends.

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

/// The write-shaped names the core's catalogue carries and a mode with no
/// consent reach hides: the workspace writers and the web pair. The old
/// session's spellings (`http_fetch`, `http_download`, `run_command`) are in no
/// list at all now, so a mode test cannot drift with them.
const WRITE_SHAPED: [&str; 5] = ["write", "edit", "notebook_edit", "web_fetch", "web_search"];

/// The spellings the core replaced: advertised by no mode, under any consent.
const OLD_SPELLINGS: [&str; 3] = ["http_fetch", "http_download", "run_command"];

/// The composed and opened run over one fixture, with the folder kept alive.
async fn compose_case(
    fixture: compose_fixture::Fixture,
    front: Arc<dyn ChatFront>,
    session: &'static str,
    mode: ApprovalPolicy,
) -> (RunComposition, RootGuard) {
    let Composed {
        application,
        run,
        root,
    } = fixture.into_parts(front, session, mode);
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

/// One turn on the composed run, waited for under the fixture's load ceiling.
async fn turn(composition: &RunComposition) {
    let turn = tokio::time::timeout(
        RUN,
        composition.job().run_next_turn(&NoopEventSink, "hello"),
    )
    .await
    .expect("the turn settles");
    assert!(turn.is_ok(), "the turn must run: {turn:?}");
}

/// A `read-only` session advertises no write-shaped tool at all: with nobody to
/// ask and no consent that reaches a write, every such tool is hidden rather
/// than offered and then denied.
#[tokio::test]
async fn a_read_only_composition_advertises_no_write_tool() {
    let provider = Scripted::start_with_script(vec![Reply::Text("hello")]);
    let fixture =
        compose_fixture::in_place_with_web_search("mode-read-only", &provider.endpoint).await;
    let front = operator_front_fixture::non_interactive();
    let (composition, _root) = compose_case(
        fixture,
        front.front(),
        "session-mode-read-only",
        ApprovalPolicy::ReadOnly,
    )
    .await;
    turn(&composition).await;

    let bodies = provider.bodies();
    let offered = advertised(&bodies[0]);
    for name in WRITE_SHAPED {
        assert!(
            !offered.iter().any(|advertised| advertised == name),
            "a read-only session offers no {name}: {offered:?}"
        );
    }
    // The read-shaped surface is untouched: the rule hides writers, not tools.
    for name in ["read", "glob", "grep", "checklist"] {
        assert!(
            offered.iter().any(|advertised| advertised == name),
            "a read-only session still offers {name}: {offered:?}"
        );
    }
    for name in OLD_SPELLINGS {
        assert!(
            !offered.iter().any(|advertised| advertised == name),
            "a read-only session offers no {name} in any case: {offered:?}"
        );
    }
}

/// The write-shaped names no consent reaches when nobody can be asked: the
/// workspace writers. The web pair is absent here too, but for its own reason
/// — nothing in this request asks for a fetch, and none is configured — so
/// asserting it would prove nothing about the mode.
const NO_PERSON_WRITE_SHAPED: [&str; 3] = ["write", "edit", "notebook_edit"];

/// A headless `ask` is the same surface: its approval is its `--allow` scopes
/// and there is nobody to put the call to, so no write-shaped tool is offered.
#[tokio::test]
async fn a_headless_ask_composition_advertises_no_write_tool() {
    let provider = Scripted::start_with_script(vec![Reply::Text("hello")]);
    let fixture = compose_fixture::in_place("mode-headless-ask", &provider.endpoint).await;
    let front = operator_front_fixture::non_interactive();
    let (composition, _root) = compose_case(
        fixture,
        front.front(),
        "session-mode-headless-ask",
        ApprovalPolicy::Ask,
    )
    .await;
    turn(&composition).await;

    let bodies = provider.bodies();
    let offered = advertised(&bodies[0]);
    for name in NO_PERSON_WRITE_SHAPED {
        assert!(
            !offered.iter().any(|advertised| advertised == name),
            "a headless ask offers no {name}: {offered:?}"
        );
    }
    for name in OLD_SPELLINGS {
        assert!(
            !offered.iter().any(|advertised| advertised == name),
            "a headless ask offers no {name} in any case: {offered:?}"
        );
    }
}

/// The rule is the mode's, not "never offer a writer": an `ask` session with a
/// person attached offers the workspace writers, because their calls can be put
/// to that person.
#[tokio::test]
async fn an_ask_composition_with_a_person_attached_offers_the_writers() {
    let provider = Scripted::start_with_script(vec![Reply::Text("hello")]);
    let fixture = compose_fixture::in_place("mode-ask-person", &provider.endpoint).await;
    let front = operator_front_fixture::interactive();
    let (composition, _root) = compose_case(
        fixture,
        front.front(),
        "session-mode-ask-person",
        ApprovalPolicy::Ask,
    )
    .await;
    turn(&composition).await;

    let bodies = provider.bodies();
    let offered = advertised(&bodies[0]);
    for name in ["write", "edit"] {
        assert!(
            offered.iter().any(|advertised| advertised == name),
            "an ask session with a person to ask offers {name}: {offered:?}"
        );
    }
}

/// The rule is read per request, so a mode the session switches reaches the
/// next request's list — the run's one cell is the advertisement's answer too.
#[tokio::test]
async fn switching_the_mode_changes_the_next_requests_surface() {
    let provider = Scripted::start_with_script(vec![Reply::Text("one"), Reply::Text("two")]);
    let fixture = compose_fixture::in_place("mode-switch", &provider.endpoint).await;
    let front = operator_front_fixture::non_interactive();
    let (composition, _root) = compose_case(
        fixture,
        front.front(),
        "session-mode-switch",
        ApprovalPolicy::ReadOnly,
    )
    .await;
    turn(&composition).await;
    composition
        .mode_cell()
        .set(ApprovalPolicy::Bypass)
        .expect("an attached composition's mode stays switchable");
    turn(&composition).await;

    let bodies = provider.bodies();
    let before = advertised(&bodies[0]);
    let after = advertised(&bodies[1]);
    assert!(
        !before.iter().any(|name| name == "write"),
        "the read-only turn offers no write: {before:?}"
    );
    assert!(
        after.iter().any(|name| name == "write"),
        "the bypass turn offers the writers again: {after:?}"
    );
}
