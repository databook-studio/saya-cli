//! A read-only (`explore`) helper's shell: sandbox-only, where nothing is
//! writable.

use std::fs;

use saya_agent::{ApprovalDecider, ApprovalPolicy, ToolError, ToolExecutor};
use serde_json::json;

use super::super::child_toolset;
use super::super::tests::{Sandbox, checkout, read_only_view};
use super::helper_shell;
use super::support::{call, host_call};
use crate::agent::tools::shell_policy;

/// A read-only (`explore`) helper's shell is sandbox-only: a call that names
/// the host is served from the frozen sandbox, where nothing is writable, and
/// it reads as usual.
#[tokio::test]
async fn a_read_only_helper_shell_serves_a_host_call_from_the_sandbox() {
    let sandbox = Sandbox::new("explore-shell");
    fs::write(sandbox.ws_root().join("existing.txt"), "readable\n").unwrap();
    let Some(shell) = helper_shell(&sandbox, ApprovalPolicy::ReadOnly, None) else {
        eprintln!("skipping `read-only`: the write-denied root does not prove on this host");
        return;
    };
    assert!(
        shell
            .approve(&shell_policy::definition(), &host_call("touch created.txt"))
            .await,
        "a host call is served from the sandbox, not refused"
    );
    let wrote = shell
        .run_call(&host_call("touch created.txt"))
        .await
        .expect("the call runs in the sandbox");
    assert_ne!(wrote["exit_code"], 0, "nothing is writable: {wrote}");
    assert_eq!(wrote["sandboxed"], true, "the host call ran sandboxed");
    assert!(!sandbox.ws_root().join("created.txt").exists());

    let read = shell
        .run_call(&call("cat existing.txt"))
        .await
        .expect("reading is still allowed");
    assert_eq!(read["exit_code"], 0, "{read}");
    assert_eq!(read["sandboxed"], true);
}

/// The read-only shape an `explore` helper holds: the workspace writes are not
/// advertised and a `write` call is refused before it can mutate anything,
/// while the shell that rides the same toolset can change nothing either.
#[tokio::test]
async fn an_explore_helper_refuses_write_and_its_shell_writes_nothing() {
    let checkout_id = checkout("explore-helper");
    let view = read_only_view(&checkout_id);
    assert!(!view.workspace_write());
    let sandbox = Sandbox::new("explore-tools");
    let pair = sandbox.pair(&checkout_id);
    let (definitions, tools) = child_toolset(&view, &pair).unwrap();
    for name in ["write", "edit"] {
        assert!(
            !definitions.iter().any(|tool| tool.name == name),
            "{name} is not advertised to a read-only helper"
        );
    }
    let refused = tools
        .execute("write", json!({ "path": "created.txt", "content": "x" }))
        .await
        .expect_err("a read-only helper cannot write");
    assert!(
        matches!(
            refused,
            ToolError::NoEffect(ref inner) if matches!(inner.as_ref(), ToolError::UnsupportedTool)
        ),
        "the refusal is the typed unsupported-tool one: {refused:?}"
    );
    assert!(!sandbox.ws_root().join("created.txt").exists());

    let Some(shell) = helper_shell(&sandbox, ApprovalPolicy::ReadOnly, None) else {
        return;
    };
    let wrote = shell
        .run_call(&host_call("touch created.txt"))
        .await
        .expect("the call is served from the sandbox");
    assert_ne!(wrote["exit_code"], 0, "nothing is writable: {wrote}");
    assert!(!sandbox.ws_root().join("created.txt").exists());
}
