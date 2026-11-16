//! A helper's background shell family: the same start/read/kill the
//! coding root advertises, owned by the helper. The two readers stay out of
//! the helper's list until it has started a run, and the run dies with the
//! helper that started it.
//!
//! The cases use only commands `/bin/sh` runs on macOS and take the host lane
//! under `bypass`; the sandbox is proven by the fixture but never asked for.

use std::sync::Arc;
use std::time::Duration;

use saya_agent::{ApprovalPolicy, ToolError, ToolExecutor};
use serde_json::json;

use super::super::child_toolset;
use super::super::tests::{Sandbox, checkout, implementer_view};
use super::helper_shell;

fn advertised(tools: &super::super::ChildTools) -> Vec<String> {
    ToolExecutor::advertised_definitions(tools)
        .expect("a helper re-states the list that can grow")
        .into_iter()
        .map(|definition| definition.name)
        .collect()
}

/// A background start made through the helper's executor answers with an id,
/// the read reaches the run, and the kill ends one that is still going. The
/// readers are unknown tools — and unadvertised — before the first start.
#[tokio::test]
async fn a_helpers_background_run_is_started_read_and_killed() {
    let checkout_id = checkout("helper-background");
    let view = implementer_view(&checkout_id);
    let sandbox = Sandbox::new("helper-background");
    let pair = sandbox.pair(&checkout_id);
    let (_definitions, mut tools) = child_toolset(&view, &pair).unwrap();
    let Some(shell) = helper_shell(&sandbox, ApprovalPolicy::Bypass, None) else {
        return;
    };
    let _ = tools.attach_shell(Arc::clone(&shell));

    let before = advertised(&tools);
    assert!(
        before.contains(&"shell".to_string()),
        "the start is advertised with the shell: {before:?}"
    );
    for name in ["shell_output", "shell_kill"] {
        assert!(
            !before.contains(&name.to_string()),
            "{name} is not advertised before a background run exists: {before:?}"
        );
        assert!(
            matches!(
                tools.execute(name, json!({"id": 1})).await,
                Err(ToolError::UnsupportedTool)
            ),
            "a call to {name} before a run exists is an unknown tool"
        );
    }

    let started = tools
        .execute(
            "shell",
            json!({"command": "sleep 5; echo late", "run_in_background": true}),
        )
        .await
        .expect("a helper's background start runs");
    let id = started["id"]
        .as_u64()
        .expect("the start answers with an id");
    assert_eq!(started["status"], "running", "the start does not wait");

    let after = advertised(&tools);
    for name in ["shell_output", "shell_kill"] {
        assert!(
            after.contains(&name.to_string()),
            "{name} joins the helper's list once a run exists: {after:?}"
        );
    }

    let read = {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        loop {
            let read = tools
                .execute("shell_output", json!({"id": id}))
                .await
                .expect("the read reaches the run");
            if read["status"] != "running" {
                break read;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "the run did not end: {read}"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    assert_eq!(read["status"], "exited");
    assert_eq!(read["exit_code"], 0);
    assert_eq!(read["stdout"], "late\n");

    let second = tools
        .execute(
            "shell",
            json!({"command": "sleep 30", "run_in_background": true}),
        )
        .await
        .expect("a second background start runs");
    let second_id = second["id"].as_u64().expect("an id");
    let killed = tools
        .execute("shell_kill", json!({"id": second_id}))
        .await
        .expect("the kill reaches the run");
    assert_eq!(killed["status"], "killed");
}

/// The helper's run ends with the helper: dropping the helper's only shell
/// kills the process group it started, so a command that would write a marker
/// after the helper is gone never writes it.
#[tokio::test]
async fn a_helpers_background_run_is_gone_after_the_helper_ends() {
    let sandbox = Sandbox::new("helper-background-lifetime");
    let marker = sandbox.ws_root().join("late.txt");
    let Some(shell) = helper_shell(&sandbox, ApprovalPolicy::Bypass, None) else {
        return;
    };
    let started = shell
        .call(
            "shell",
            &json!({"command": "sleep 1; echo late > late.txt", "run_in_background": true}),
        )
        .await
        .expect("the helper starts a background run");
    assert_eq!(started["status"], "running", "{started}");

    // The helper ends: nothing else holds its shell.
    drop(shell);
    tokio::time::sleep(Duration::from_millis(2000)).await;
    assert!(
        !marker.exists(),
        "the run's process group died with the helper"
    );
}
