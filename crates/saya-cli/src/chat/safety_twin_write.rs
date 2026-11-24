//! Twins of `interactive/session/universe/tests/workspace_write_effects.rs` on
//! the live chat path: a `write` call the model issues, through a session's run
//! whose launch states the workspace-write grant (or does not).
//!
//! The old tests drove a headless ask session whose live policy held the grant
//! and whose decider could not ask. The live twin states the same two facts the
//! only way the chat has them: the grant rides the run's launch store
//! (`with_launch_grants`, exactly what `line_chat::open_run` states for
//! `--allow`), and the front is the headless one — nobody to ask, so no tool
//! whose call it could not ask about is offered.

use std::sync::Arc;

use saya_agent::ApprovalPolicy;
use serde_json::json;

use super::ChatFront;
use super::compose_fixture;
use super::safety_twin_fixture::{
    Fixed, WORKSPACE_WRITE, advertised, compose, offers, result_of, turn,
};
use crate::commands::coding::parent_turn_provider::{Reply, Scripted};

/// The file a granted write leaves behind, and the content it must hold
/// exactly once.
const GRANTED: &str = "grant-headless.txt";
const CONTENT: &str = "written under the grant\n";

/// The path outside the run's write scope: the working checkout's own parent,
/// which no call of this run may write.
const OUTSIDE: &str = "../escaped.txt";

/// Under the grant the call runs and the file lands, once; and a path outside
/// the run's scope is refused even then — the grant widens the *consent*, not
/// the containment.
#[tokio::test]
async fn a_headless_ask_session_runs_a_workspace_write_call_under_its_grant() {
    let provider = Scripted::start_with_script(vec![
        Reply::Call {
            name: "write",
            arguments: json!({"path": GRANTED, "content": CONTENT}),
        },
        Reply::Call {
            name: "write",
            arguments: json!({"path": OUTSIDE, "content": "must not be written\n"}),
        },
        Reply::Text("done"),
    ]);
    let fixture = compose_fixture::in_place("twin-write-granted", &provider.endpoint).await;
    let front = Fixed::headless();
    let (composition, root) = compose(
        fixture,
        Arc::clone(&front) as Arc<dyn ChatFront>,
        "twin-write-granted",
        ApprovalPolicy::Ask,
        &[WORKSPACE_WRITE],
    )
    .await;

    let _ = turn(&composition, "write the file").await;

    let bodies = provider.bodies();
    assert!(
        offers(&advertised(&bodies[0]), "write"),
        "the held grant advertises the write family: {:?}",
        advertised(&bodies[0])
    );
    let landed = root.checkout_root().join(GRANTED);
    assert_eq!(
        std::fs::read_to_string(&landed).expect("the granted call wrote through the seam"),
        CONTENT,
        "the granted call landed exactly once"
    );
    let refused = result_of(&bodies, "call-1");
    assert!(
        refused.get("error").is_some(),
        "a write outside the run's scope is refused: {refused}"
    );
    assert!(
        !root
            .checkout_root()
            .parent()
            .unwrap()
            .join("escaped.txt")
            .exists(),
        "the refused call wrote nothing outside the scope"
    );
    assert_eq!(
        front.seen(),
        0,
        "the launch's own grant pre-answers the call, so nobody is asked"
    );
}

/// The closed shape: without the grant the family stays hidden, the model's
/// `write` call fails as an unknown tool, and the workspace is untouched.
#[tokio::test]
async fn a_headless_ask_session_without_the_grant_refuses_the_workspace_write_call() {
    let provider = Scripted::start_with_script(vec![
        Reply::Call {
            name: "write",
            arguments: json!({"path": GRANTED, "content": "must not be written\n"}),
        },
        Reply::Text("done"),
    ]);
    let fixture = compose_fixture::in_place("twin-write-closed", &provider.endpoint).await;
    let front = Fixed::headless();
    let (composition, root) = compose(
        fixture,
        Arc::clone(&front) as Arc<dyn ChatFront>,
        "twin-write-closed",
        ApprovalPolicy::Ask,
        &[],
    )
    .await;

    let _ = turn(&composition, "write the file").await;

    let bodies = provider.bodies();
    assert!(
        !offers(&advertised(&bodies[0]), "write"),
        "no grant, no advertisement: {:?}",
        advertised(&bodies[0])
    );
    let refused = result_of(&bodies, "call-0");
    assert!(
        refused["error"]
            .as_str()
            .unwrap_or_default()
            .contains("unknown tool 'write'"),
        "the refused call was answered as an unknown tool: {refused}"
    );
    assert!(
        !root.checkout_root().join(GRANTED).exists(),
        "the workspace was never written"
    );
    assert_eq!(front.seen(), 0, "nobody can be asked, so nobody was");
}
