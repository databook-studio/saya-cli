//! The root's background shell family: a background call takes exactly the
//! lane decision a foreground one takes — the same ask, the same sandbox — and
//! a run started through the tool is read and killed through the tool. Which of
//! the family the root advertises, and when, lives in
//! [`super::root_shell_advertisement_tests`].
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::sync::Arc;

use saya_agent::{ApprovalDecider, ApprovalPolicy};
use saya_harness::runner::HostPolicy;
use serde_json::json;

use super::root_shell::{self, RootShell};
use super::root_shell_tests::fixture::{RecordingOperator, TempCheckout, host_shell_over, prompt};
use crate::agent::tools::consent;
use crate::agent::tools::shell_policy;

/// A shell over `checkout` under `policy`, or `None` where this host proves no
/// sandbox.
pub(super) fn shell_over(
    checkout: &TempCheckout,
    policy: ApprovalPolicy,
    operator: Option<Arc<dyn consent::Operator>>,
) -> Option<RootShell> {
    root_shell::build_with_policy(
        checkout.path(),
        policy,
        prompt(operator),
        saya_agent::CancellationToken::new(),
    )
}

/// The two readers the root advertises beside `shell`.
pub(super) fn reader(name: &str) -> saya_agent::ToolDefinition {
    shell_policy::background_definitions(shell_policy::BackgroundOwner::Job)
        .into_iter()
        .find(|definition| definition.name == name)
        .expect("the family advertises both readers")
}

/// A background host call is a shell call for policy purposes: under `ask` it
/// reaches the operator exactly as a foreground one does, and the answer
/// decides.
#[tokio::test]
async fn a_background_host_call_asks_the_operator_exactly_as_a_foreground_one() {
    let checkout = TempCheckout::new("root-background-ask");
    let operator = RecordingOperator::new(true);
    let Some(shell) = shell_over(&checkout, ApprovalPolicy::Ask, Some(operator.clone())) else {
        return;
    };
    let definition =
        shell_policy::background_definitions(shell_policy::BackgroundOwner::Job).remove(0);
    let call = json!({"command": "true", "run_in_background": true});
    assert!(
        shell.approve(&definition, &call).await,
        "the operator allowed the call"
    );
    assert_eq!(
        operator.calls(),
        1,
        "a background host call asks exactly once, as a foreground one does"
    );

    let refusing = RecordingOperator::new(false);
    let Some(shell) = shell_over(&checkout, ApprovalPolicy::Ask, Some(refusing.clone())) else {
        return;
    };
    assert!(
        !shell.approve(&definition, &call).await,
        "the operator's refusal is the call's refusal"
    );
    assert_eq!(refusing.calls(), 1);
}

/// A call that names the sandbox runs there without asking, background or
/// not: the confinement is the consent in both shapes.
#[tokio::test]
async fn a_sandboxed_background_start_never_asks() {
    let checkout = TempCheckout::new("root-background-sandbox");
    let operator = RecordingOperator::new(true);
    let Some(shell) = shell_over(&checkout, ApprovalPolicy::Ask, Some(operator.clone())) else {
        return;
    };
    let definition =
        shell_policy::background_definitions(shell_policy::BackgroundOwner::Job).remove(0);
    let call = json!({"command": "true", "run_in_background": true, "sandbox": true});
    assert!(shell.approve(&definition, &call).await);
    assert_eq!(operator.calls(), 0, "the sandbox lane never asks");
}

/// Reading and killing carry no new authority: neither reaches the operator,
/// and neither is refused by the lane the start already took.
#[tokio::test]
async fn the_readers_never_reach_the_operator() {
    let checkout = TempCheckout::new("root-background-readers");
    let operator = RecordingOperator::new(true);
    let Some(shell) = shell_over(&checkout, ApprovalPolicy::Ask, Some(operator.clone())) else {
        return;
    };
    assert!(
        shell
            .approve(&reader("shell_output"), &json!({"id": 1}))
            .await
    );
    assert!(
        shell
            .approve(&reader("shell_kill"), &json!({"id": 1}))
            .await
    );
    assert_eq!(
        operator.calls(),
        0,
        "a reader reaches only runs this shell started"
    );
}

/// Reads `id` until the run ends, accumulating the stdout each read returned.
/// A read answers as soon as the run writes, so a run that writes before it
/// ends — including a sandbox that reports a startup message — takes more than
/// one read to reach its exit.
async fn read_until_exit(shell: &RootShell, id: u64) -> (String, serde_json::Value) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
    let mut stdout = String::new();
    loop {
        let read = shell
            .call("shell_output", &json!({"id": id}))
            .await
            .expect("the run is readable");
        if let Some(text) = read["stdout"].as_str() {
            stdout.push_str(text);
        }
        if read["status"] != "running" {
            return (stdout, read);
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the run did not end: {read}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// The whole path through the tool: a detached `sleep 5; echo late` answers
/// with an id at once, the read waits for the line, and the kill ends a run
/// that is still going.
#[tokio::test]
async fn a_run_started_through_the_tool_is_read_and_killed() {
    let checkout = TempCheckout::new("root-background-round-trip");
    let Some(shell) = shell_over(&checkout, ApprovalPolicy::Bypass, None) else {
        return;
    };
    let started = shell
        .call(
            "shell",
            &json!({"command": "sleep 5; echo late", "run_in_background": true, "sandbox": true}),
        )
        .await
        .expect("a background start runs");
    let id = started["id"]
        .as_u64()
        .expect("the start answers with an id");
    assert_eq!(started["status"], "running", "the start does not wait");
    assert_eq!(started["stdout"], "");

    let (stdout, read) = read_until_exit(&shell, id).await;
    assert_eq!(read["status"], "exited");
    assert_eq!(read["exit_code"], 0);
    assert_eq!(stdout, "late\n");

    let second = shell
        .call(
            "shell",
            &json!({"command": "sleep 30", "run_in_background": true, "sandbox": true}),
        )
        .await
        .expect("a second background start runs");
    let second_id = second["id"].as_u64().expect("an id");
    let killed = shell
        .call("shell_kill", &json!({"id": second_id}))
        .await
        .expect("the kill reaches the run");
    assert_eq!(killed["status"], "killed");
}

/// A foreground call that outlives its timeout is moved to the background,
/// not killed: the tool answers with the id and a notice, and the run is
/// readable afterwards through the same `shell_output` path.
#[tokio::test]
async fn a_timed_out_foreground_call_is_moved_to_the_background() {
    let checkout = TempCheckout::new("root-timeout-moves");
    let Some(shell) = shell_over(&checkout, ApprovalPolicy::Bypass, None) else {
        return;
    };
    let moved = shell
        .call(
            "shell",
            &json!({"command": "sleep 2; echo done", "sandbox": true, "timeout": 1000}),
        )
        .await
        .expect("a timed-out call moves to the background instead of failing");
    let id = moved["id"]
        .as_u64()
        .expect("a moved run answers with an id");
    assert_eq!(
        moved["status"], "running",
        "the run is still going: {moved}"
    );
    assert_eq!(moved["timed_out"], true, "the timeout did fire: {moved}");
    let notice = moved["notice"].as_str().expect("a notice names the move");
    assert!(
        notice.contains("moved to background after 1 s"),
        "the notice states when and where: {notice}"
    );
    assert!(
        notice.contains("shell_output"),
        "the notice names the reader: {notice}"
    );

    let (stdout, read) = read_until_exit(&shell, id).await;
    assert_eq!(read["status"], "exited");
    assert_eq!(read["exit_code"], 0);
    assert_eq!(stdout, "done\n");
}

/// The background cap bounds the move too: with every slot held, a timed-out
/// foreground call keeps today's kill and `timed_out`, and returns no id.
#[tokio::test]
async fn a_timed_out_call_with_the_background_cap_full_is_killed() {
    let checkout = TempCheckout::new("root-timeout-cap");
    let Some(shell) = shell_over(&checkout, ApprovalPolicy::Bypass, None) else {
        return;
    };
    let mut held = Vec::new();
    for _ in 0..saya_harness::runner::MAX_BACKGROUND_RUNS {
        let started = shell
            .call(
                "shell",
                &json!({"command": "sleep 30", "run_in_background": true, "sandbox": true}),
            )
            .await
            .expect("a background start runs");
        held.push(started["id"].as_u64().expect("an id"));
    }
    let timed_out = shell
        .call(
            "shell",
            &json!({"command": "sleep 2; echo never", "sandbox": true, "timeout": 1000}),
        )
        .await
        .expect("the call still answers");
    assert_eq!(
        timed_out["timed_out"], true,
        "a full cap keeps the kill: {timed_out}"
    );
    assert!(
        timed_out.get("id").is_none(),
        "a full cap returns no background id: {timed_out}"
    );
    for id in held {
        let _ = shell.call("shell_kill", &json!({"id": id})).await;
    }
}

/// A read of a run this shell never started is the typed refusal, never an
/// empty answer the model could read as "no output yet".
#[tokio::test]
async fn reading_an_unknown_id_is_refused() {
    let checkout = TempCheckout::new("root-background-unknown");
    let Some(shell) = shell_over(&checkout, ApprovalPolicy::Bypass, None) else {
        return;
    };
    let refused = shell.call("shell_output", &json!({"id": 99})).await;
    assert!(refused.is_err(), "an unknown id is refused: {refused:?}");
}

/// The host lane is a shell lane too: a background run there is killed by the
/// job, exactly as a sandboxed one is.
#[tokio::test]
async fn a_host_background_run_is_killed_by_the_tool() {
    let checkout = TempCheckout::new("root-background-host");
    let Some(shell) = host_shell_over(
        checkout.path(),
        ApprovalPolicy::Bypass,
        None,
        HostPolicy::default(),
    ) else {
        return;
    };
    let started = shell
        .call(
            "shell",
            &json!({"command": "sleep 30", "run_in_background": true, "sandbox": false}),
        )
        .await
        .expect("a host background start runs");
    let id = started["id"]
        .as_u64()
        .expect("the start answers with an id");
    assert_eq!(started["sandboxed"], false, "the run took the host lane");
    let killed = shell
        .call("shell_kill", &json!({"id": id}))
        .await
        .expect("the kill reaches the run");
    assert_eq!(killed["status"], "killed");
}

/// A headless `ask` start takes the lane decision a foreground call takes,
/// notice and all: with no operator to answer it, a host call is rerouted to
/// the proven sandbox, and the start's own result says where it ran and why.
#[tokio::test]
async fn a_headless_background_start_carries_the_sandbox_notice() {
    let checkout = TempCheckout::new("root-background-notice");
    let Some(shell) = shell_over(&checkout, ApprovalPolicy::Ask, None) else {
        return;
    };
    let started = shell
        .call(
            "shell",
            &json!({"command": "sleep 30", "run_in_background": true, "sandbox": false}),
        )
        .await
        .expect("the rerouted start runs");
    assert_eq!(
        started["sandboxed"], true,
        "the headless start took the sandbox lane: {started}"
    );
    assert_eq!(
        started["lane_notice"].as_str(),
        Some(shell_policy::SANDBOX_DOWNGRADE_NOTICE),
        "the start says where it ran and why: {started}"
    );
    let id = started["id"]
        .as_u64()
        .expect("the start answers with an id");
    let killed = shell
        .call("shell_kill", &json!({"id": id}))
        .await
        .expect("the kill reaches the run");
    assert_eq!(killed["status"], "killed");
}
