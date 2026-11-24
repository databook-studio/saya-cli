//! Twins of `interactive/session/universe/tests/shell_turn.rs` on the live
//! chat path: a `shell` call the model issues in a turn of a composed session's
//! run, decided by the surface the run puts its questions to.
//!
//! The old tests stated the call to `TerminalApproval::from_session_…` over a
//! `SessionUniverse` that advertised the tool. The live run is the same
//! guarantee with the live names: the call is `shell` (the job root's one
//! catalogue name), the decider is [`ChatFront::ask`] through the run's
//! operator, and the decision is read back from the turn's own events.

use std::sync::Arc;

use saya_agent::{AgentEvent, ApprovalPolicy};
use serde_json::json;

use super::ChatFront;
use super::compose_fixture;
use super::safety_twin_fixture::{Fixed, compose, shell_proves, turn};
use crate::agent::tools::consent::Answer;
use crate::commands::coding::parent_turn_provider::{Reply, Scripted};

/// The marker a refused call would leave if it had run: inside the run's own
/// checkout, where its shell composes.
const MARKER: &str = "refused-shell.txt";

/// The front's refusal is the call's refusal: the host shell call is put to it
/// exactly once, the turn carries the denial at the call boundary, and the
/// command never ran — the folder the call would have written holds no marker.
#[tokio::test]
async fn a_host_shell_call_the_front_refuses_is_denied_and_never_runs() {
    let provider = Scripted::start_with_script(vec![
        Reply::Call {
            name: "shell",
            arguments: json!({"command": format!("touch {MARKER}")}),
        },
        Reply::Text("done"),
    ]);
    let fixture = compose_fixture::in_place("twin-shell-refused", &provider.endpoint).await;
    let front = Fixed::new(Answer::Deny);
    let (composition, root) = compose(
        fixture,
        Arc::clone(&front) as Arc<dyn ChatFront>,
        "twin-shell-refused",
        ApprovalPolicy::Ask,
        &[],
    )
    .await;

    let output = turn(&composition, "run it").await;

    assert_eq!(
        front.seen(),
        1,
        "the host shell call was put to the session's approval surface exactly once"
    );
    assert!(
        output.events.iter().any(|event| matches!(
            event,
            AgentEvent::ToolDenied { name, .. } if name == "shell"
        )),
        "the surface's refusal denied the shell call at the call boundary: {:?}",
        output.events
    );
    assert!(
        !root.checkout_root().join(MARKER).exists(),
        "a refused shell call never ran: the marker was not written"
    );
}

/// The same turn with the call naming the sandbox: the confinement is the
/// consent, so the surface is never asked and the call runs. Skipped where this
/// host proves no sandbox — the composition's own probe decides, as it did on
/// the old path.
#[tokio::test]
async fn a_sandboxed_shell_call_runs_without_asking() {
    let provider = Scripted::start_with_script(vec![
        Reply::Call {
            name: "shell",
            arguments: json!({"command": "true", "sandbox": true}),
        },
        Reply::Text("done"),
    ]);
    let fixture = compose_fixture::in_place("twin-shell-sandbox", &provider.endpoint).await;
    let front = Fixed::new(Answer::Deny);
    let (composition, root) = compose(
        fixture,
        Arc::clone(&front) as Arc<dyn ChatFront>,
        "twin-shell-sandbox",
        ApprovalPolicy::Ask,
        &[],
    )
    .await;
    if !shell_proves(&root.checkout_root(), ApprovalPolicy::Ask) {
        eprintln!("skipping `a_sandboxed_shell_call_runs_without_asking`: no sandbox");
        return;
    }

    let output = turn(&composition, "run it").await;

    assert_eq!(
        front.seen(),
        0,
        "the sandbox lane needs no approval surface"
    );
    assert!(
        output.events.iter().any(|event| matches!(
            event,
            AgentEvent::ToolCompleted { name, .. } if name == "shell"
        )),
        "the sandboxed shell call ran: {:?}",
        output.events
    );
}
