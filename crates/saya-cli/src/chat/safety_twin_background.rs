//! Twins of `interactive/session/universe/tests/shell_background.rs` on the
//! live chat path: the run's own `shell` family started detached through a
//! turn, read and killed through later ones.
//!
//! The old tests composed the session's executor and its own shell per turn,
//! and pinned that the session — not the turn — owned the run registry. On the
//! live path the run's job owns the shell for the whole run, so a detached
//! command started in one turn is reachable from the next: the same guarantee,
//! at the run's own scope. The reader advertisement grows exactly as it did:
//! no `shell_output`/`shell_kill` before a start exists, both after one.

use std::sync::Arc;

use saya_agent::{AgentEvent, ApprovalPolicy};
use serde_json::json;

use super::ChatFront;
use super::compose_fixture;
use super::safety_twin_fixture::{Fixed, WAIT, advertised, compose, offers, result_of, turn};
use crate::agent::tools::consent::Answer;
use crate::commands::coding::parent_turn_provider::{Reply, Scripted};

/// A command that keeps running for a moment, then leaves one line behind:
/// the marker lets the test wait for the work without spending a turn on it.
const BACKGROUND_THEN_ECHO: &str = "sleep 1; echo late > late.txt; echo late";

/// A command that is still running when the run kills it.
const LONG_RUN: &str = "sleep 30";

/// The start answers with an id, the read reaches the run, and the kill ends
/// one that is still going. The two readers are unknown before the first start
/// and advertised after it, and only a start reaches the front: reading and
/// killing carry no new authority.
#[tokio::test]
async fn a_chat_background_run_is_started_read_and_killed() {
    let provider = Scripted::start_with_script(vec![
        Reply::Call {
            name: "shell",
            arguments: json!({"command": BACKGROUND_THEN_ECHO, "run_in_background": true}),
        },
        Reply::Text("started"),
        Reply::Call {
            name: "shell_output",
            arguments: json!({"id": 1}),
        },
        Reply::Text("read"),
        Reply::Call {
            name: "shell",
            arguments: json!({"command": LONG_RUN, "run_in_background": true}),
        },
        // The kill rides the same turn as the start: a turn that is still
        // calling tools is not waiting for an exit, so the model reaches a run
        // that is still going.
        Reply::Call {
            name: "shell_kill",
            arguments: json!({"id": 2}),
        },
        Reply::Text("killed"),
    ]);
    let fixture = compose_fixture::in_place("twin-background", &provider.endpoint).await;
    let front = Fixed::new(Answer::Once);
    let (composition, root) = compose(
        fixture,
        Arc::clone(&front) as Arc<dyn ChatFront>,
        "twin-background",
        ApprovalPolicy::Ask,
        &[],
    )
    .await;

    let first = turn(&composition, "start it").await;
    assert!(
        first.events.iter().any(|event| matches!(
            event,
            AgentEvent::ToolCompleted { name, .. } if name == "shell"
        )),
        "the start ran: {:?}",
        first.events
    );
    let bodies = provider.bodies();
    let before = advertised(&bodies[0]);
    assert!(
        offers(&before, "shell"),
        "the start is advertised with the shell: {before:?}"
    );
    for name in ["shell_output", "shell_kill"] {
        assert!(
            !offers(&before, name),
            "{name} is not advertised before a background run exists: {before:?}"
        );
    }
    let started = result_of(&bodies, "call-0");
    assert_eq!(started["status"], "running", "the start does not wait");
    assert_eq!(started["id"], 1, "the start answers with the run's id");
    let after = advertised(&bodies[1]);
    for name in ["shell_output", "shell_kill"] {
        assert!(
            offers(&after, name),
            "{name} joins the list once a run exists: {after:?}"
        );
    }

    wait_for(&root.checkout_root().join("late.txt"));

    let _ = turn(&composition, "read it").await;
    let read = result_of(&provider.bodies(), "call-2");
    assert_eq!(read["status"], "exited", "the read reached the run: {read}");
    assert_eq!(read["exit_code"], 0, "the run ended cleanly: {read}");
    assert!(
        read["stdout"].as_str().unwrap_or_default().contains("late"),
        "the read carried the line the run wrote: {read}"
    );

    let _ = turn(&composition, "start a long one and kill it").await;
    let second = result_of(&provider.bodies(), "call-4");
    assert_eq!(
        second["status"], "running",
        "the second start does not wait: {second}"
    );
    assert_eq!(second["id"], 2, "the second run has its own id: {second}");
    let killed = result_of(&provider.bodies(), "call-5");
    assert_eq!(
        killed["status"], "killed",
        "the kill ended the run that was still going: {killed}"
    );
    assert_eq!(
        front.seen(),
        2,
        "each start is a call somebody answers; the two readers are not"
    );
}

/// A run one turn started is readable from the next turn: the run owns the
/// registry, so the turn that started it ending does not end the command.
#[tokio::test]
async fn a_chat_background_run_outlives_the_turn_that_started_it() {
    let provider = Scripted::start_with_script(vec![
        Reply::Call {
            name: "shell",
            arguments: json!({"command": BACKGROUND_THEN_ECHO, "run_in_background": true}),
        },
        Reply::Text("started"),
        Reply::Call {
            name: "shell_output",
            arguments: json!({"id": 1}),
        },
        Reply::Text("read"),
    ]);
    let fixture = compose_fixture::in_place("twin-background-turn", &provider.endpoint).await;
    let front = Fixed::new(Answer::Once);
    let (composition, root) = compose(
        fixture,
        Arc::clone(&front) as Arc<dyn ChatFront>,
        "twin-background-turn",
        ApprovalPolicy::Ask,
        &[],
    )
    .await;

    let _ = turn(&composition, "start it").await;
    assert_eq!(
        result_of(&provider.bodies(), "call-0")["id"],
        1,
        "the first turn started the run"
    );
    wait_for(&root.checkout_root().join("late.txt"));

    let _ = turn(&composition, "read it").await;
    let read = result_of(&provider.bodies(), "call-2");
    assert_eq!(
        read["status"], "exited",
        "the next turn reached the run the first one started: {read}"
    );
    assert!(
        read["stdout"].as_str().unwrap_or_default().contains("late"),
        "the next turn read the line the run wrote: {read}"
    );
}

/// A mode that offers no shell refuses a background start like any other shell
/// call — before any approval surface is consulted, with no run recorded for a
/// later turn to read. The mode is `never`: the chat composes no plan-first run
/// (see REPORT.md's row for `plan_mode_refuses_a_background_start`).
#[tokio::test]
async fn a_mode_that_offers_no_shell_refuses_a_background_start() {
    let provider = Scripted::start_with_script(vec![
        Reply::Call {
            name: "shell",
            arguments: json!({"command": BACKGROUND_THEN_ECHO, "run_in_background": true}),
        },
        Reply::Text("done"),
    ]);
    let fixture = compose_fixture::in_place("twin-background-closed", &provider.endpoint).await;
    let front = Fixed::new(Answer::Once);
    let (composition, root) = compose(
        fixture,
        Arc::clone(&front) as Arc<dyn ChatFront>,
        "twin-background-closed",
        ApprovalPolicy::Never,
        &[],
    )
    .await;

    let _ = turn(&composition, "start it").await;

    assert_eq!(
        front.seen(),
        0,
        "a mode that offers no shell refuses before any approval surface is consulted"
    );
    let bodies = provider.bodies();
    assert!(
        !offers(&advertised(&bodies[0]), "shell"),
        "the shell is not advertised under this mode: {:?}",
        advertised(&bodies[0])
    );
    let refused = result_of(&bodies, "call-0");
    assert!(
        refused["error"]
            .as_str()
            .unwrap_or_default()
            .contains("unknown tool 'shell'"),
        "the refused start was answered as an unknown tool: {refused}"
    );
    let after = advertised(&bodies[1]);
    for name in ["shell_output", "shell_kill"] {
        assert!(
            !offers(&after, name),
            "no run was recorded by a refused start: {after:?}"
        );
    }
    assert!(
        !root.checkout_root().join("late.txt").exists(),
        "a refused start never ran its command"
    );
}

/// Waits for `path` to exist, under the fixture's load ceiling: a detached
/// command writes it when its work is done.
fn wait_for(path: &std::path::Path) {
    let deadline = std::time::Instant::now() + WAIT;
    while !path.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "the background run never wrote {}",
            path.display()
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}
