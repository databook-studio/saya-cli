//! Twin of the `never` posture's advertisement, deleted with the old universe
//! (`universe/tests/policy_advertisement.rs` and `mode_tool_surfaces.rs`).
//!
//! `compose_mode_tests` pins read-only, the headless ask, an ask with a person
//! and a mode switch. `never` — the posture where no consent reaches a writer
//! at all — was only ever pinned on the deleted universe. The live chat
//! composes the posture, so the same outcome is stated here: no write-shaped
//! tool is offered, in any spelling the core knows, while the read-shaped
//! surface is untouched.
//!
//! The other deleted advertisement row — a run outside any workspace
//! (`worktree_tool_surfaces.rs`) — already has a live pin on the line path:
//! `line_chat/one_shot_tests.rs::ask_from_home_advertises_no_workspace_tool`
//! and `line_chat/workspace_tests.rs::
//! a_session_over_a_folder_that_holds_its_state_root_advertises_no_workspace_tool`.
//! The chat fixture opens only over a root it can bind, so this file does not
//! restate that row.

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

/// The write-shaped names the core's catalogue carries: hidden wherever no
/// consent reaches them, in the spellings the core uses.
const WRITE_SHAPED: [&str; 5] = ["write", "edit", "notebook_edit", "web_fetch", "web_search"];

/// The composed and opened run, with the folder kept alive.
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

/// A `never` session — no consent posture at all — advertises no write-shaped
/// tool, in any spelling the core knows. The read-shaped surface is untouched:
/// the posture withholds writers, not reads.
#[tokio::test]
async fn a_never_composition_advertises_no_write_tool() {
    let provider = Scripted::start_with_script(vec![Reply::Text("hello")]);
    let fixture = compose_fixture::in_place_with_web_search("mode-never", &provider.endpoint).await;
    let front = operator_front_fixture::non_interactive();
    let (composition, _root) = compose_case(
        fixture,
        front.front(),
        "session-mode-never",
        ApprovalPolicy::Never,
    )
    .await;
    turn(&composition).await;

    let bodies = provider.bodies();
    let offered = advertised(&bodies[0]);
    for name in WRITE_SHAPED {
        assert!(
            !offered.iter().any(|advertised| advertised == name),
            "a `never` session offers no {name}: {offered:?}"
        );
    }
    for name in ["read", "glob", "grep"] {
        assert!(
            offered.iter().any(|advertised| advertised == name),
            "a `never` session still offers the read-shaped {name}: {offered:?}"
        );
    }
}
