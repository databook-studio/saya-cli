//! The job's one permission-mode cell, read per helper call: a mode the job
//! states while a helper runs decides that helper's next `shell` call, exactly
//! as it decides the root's — and a helper type that cannot write stays
//! read-only however wide the cell moves.

use saya_agent::{ApprovalDecider, ApprovalPolicy};

use super::super::tests::Sandbox;
use super::support::{RecordingOperator, call, helper_shell_following_mode, sandbox_call};
use crate::agent::tools::shell_policy;

/// The defect these tests pin: a helper's shell copied the job's mode at child
/// composition, so a switch made while the helper ran reached nothing until the
/// helper was composed again.
///
/// A job under `ask` with a running helper: the job's cell moves to
/// `read-only` between two of the *helper's* shell calls, and the second call
/// is answered exactly as a read-only helper's shell answers it — not run on
/// the host, and never a second question to the operator.
#[tokio::test]
async fn a_mode_stated_between_two_helper_calls_decides_the_second() {
    let sandbox = Sandbox::new("helper-mode-cell");
    let operator = RecordingOperator::new(true);
    let mode = shell_policy::ModeCell::new(ApprovalPolicy::Ask);
    let Some(shell) = helper_shell_following_mode(
        &sandbox,
        &mode,
        ApprovalPolicy::Bypass,
        Some(operator.clone()),
    ) else {
        return;
    };
    let definition = shell_policy::definition();

    // Under `ask`, the helper's host call is put to the job's operator.
    assert!(shell.approve(&definition, &call("true")).await);
    assert_eq!(
        operator.calls(),
        1,
        "the asking mode asks the job's operator"
    );
    let first = shell
        .run_call(&call("true"))
        .await
        .expect("the approved host call runs");
    assert_eq!(
        first["sandboxed"], false,
        "the asking mode ran it on the host: {first}"
    );

    mode.set(ApprovalPolicy::ReadOnly)
        .expect("an unfrozen cell takes a change");

    // The helper's next call resolves the job's current mode: read-only serves
    // a host call from the sandbox where one is proven and refuses it where
    // none is. Either way it is not run on the host, and it is not asked about.
    if shell_policy::prove(&sandbox.ws_root(), ApprovalPolicy::Ask).is_some() {
        assert!(
            shell.approve(&definition, &call("true")).await,
            "read-only serves the call from the sandbox"
        );
        assert_eq!(operator.calls(), 1, "read-only never asks");
        let second = shell
            .run_call(&call("true"))
            .await
            .expect("the sandbox lane runs the call");
        assert_eq!(
            second["sandboxed"], true,
            "the helper's next call followed the moved cell: {second}"
        );
    } else {
        assert!(
            !shell.approve(&definition, &call("true")).await,
            "read-only refuses the host lane"
        );
        assert_eq!(operator.calls(), 1, "read-only never asks");
        let refused = shell
            .run_call(&call("true"))
            .await
            .expect_err("the call is not run at all");
        assert!(
            refused.to_string().contains("read-only"),
            "the refusal is read-only's own: {refused}"
        );
    }
}

/// A switchable job moves its cell to `bypass` while a helper composed under
/// `ask` runs: the helper's next call follows the cell, so it is no longer put
/// to the operator.
#[tokio::test]
async fn a_switch_to_bypass_reaches_a_helper_that_may_write() {
    let sandbox = Sandbox::new("helper-mode-cell-bypass");
    let operator = RecordingOperator::new(false);
    let mode = shell_policy::ModeCell::new(ApprovalPolicy::Ask);
    let Some(shell) = helper_shell_following_mode(
        &sandbox,
        &mode,
        ApprovalPolicy::Bypass,
        Some(operator.clone()),
    ) else {
        return;
    };
    let definition = shell_policy::definition();

    assert!(
        !shell.approve(&definition, &call("true")).await,
        "the operator refuses the host call under ask"
    );
    assert_eq!(operator.calls(), 1);

    mode.set(ApprovalPolicy::Bypass)
        .expect("a switchable job takes a change");

    assert!(
        shell.approve(&definition, &call("true")).await,
        "the switched helper's next call follows the cell"
    );
    assert_eq!(operator.calls(), 1, "bypass never asks");
    let ran = shell
        .run_call(&call("true"))
        .await
        .expect("bypass runs the call on the host");
    assert_eq!(
        ran["sandboxed"], false,
        "the helper's host lane followed the cell: {ran}"
    );
}

/// Capability narrowing is applied *after* the mode: a read-only (`explore`)
/// helper type composed under `ask` stays read-only when the job's cell moves
/// to `bypass`, so the switch can never widen it past its own capability.
#[tokio::test]
async fn a_read_only_helper_type_stays_read_only_when_the_job_switches_to_bypass() {
    let sandbox = Sandbox::new("helper-mode-cell-cap");
    let operator = RecordingOperator::new(true);
    let mode = shell_policy::ModeCell::new(ApprovalPolicy::Ask);
    let Some(shell) = helper_shell_following_mode(
        &sandbox,
        &mode,
        ApprovalPolicy::ReadOnly,
        Some(operator.clone()),
    ) else {
        return;
    };

    mode.set(ApprovalPolicy::Bypass)
        .expect("a switchable job takes a change");

    assert_eq!(
        mode.get(),
        ApprovalPolicy::Bypass,
        "the job's own cell took the switch"
    );
    if shell_policy::prove(&sandbox.ws_root(), ApprovalPolicy::Ask).is_some() {
        let ran = shell
            .run_call(&call("true"))
            .await
            .expect("the sandbox lane serves a read-only type");
        assert_eq!(
            ran["sandboxed"], true,
            "a read-only type is never handed the host lane: {ran}"
        );
    } else {
        let refused = shell
            .run_call(&call("true"))
            .await
            .expect_err("a read-only type with no sandbox is served nothing");
        assert!(
            refused.to_string().contains("read-only"),
            "the refusal is read-only's own: {refused}"
        );
    }
    assert_eq!(
        operator.calls(),
        0,
        "a read-only helper's call is never put to the operator, bypass cell or not"
    );
}

/// A frozen job keeps its helper's mode: the cell refuses the change, and the
/// helper's shell resolves the mode the launch stated — a detached `saya -p`,
/// `saya start` or `saya job` run has no operator a switch could be put to.
#[tokio::test]
async fn a_frozen_job_refuses_the_change_and_its_helpers_mode_is_unchanged() {
    let sandbox = Sandbox::new("helper-mode-cell-frozen");
    let operator = RecordingOperator::new(true);
    let mode = shell_policy::ModeCell::new(ApprovalPolicy::Ask);
    mode.freeze_at(ApprovalPolicy::Ask);
    let Some(shell) = helper_shell_following_mode(
        &sandbox,
        &mode,
        ApprovalPolicy::Bypass,
        Some(operator.clone()),
    ) else {
        return;
    };
    let definition = shell_policy::definition();

    assert!(
        mode.set(ApprovalPolicy::Bypass).is_err(),
        "a frozen job refuses a mode change"
    );
    assert_eq!(
        mode.get(),
        ApprovalPolicy::Ask,
        "and the cell keeps the mode the launch stated"
    );
    assert!(
        shell.approve(&definition, &call("true")).await,
        "the helper's call is still the asking mode's"
    );
    assert_eq!(
        operator.calls(),
        1,
        "the asking mode still asks, exactly as before the refused switch"
    );
}

/// The capability narrows the composition as well as the mode it resolves: a
/// read-only type composed while the job's cell is `bypass` still gets the
/// write-denied sandbox, so nothing it runs can change the shared checkout —
/// the one part of `read-only` that is a property of the sandbox, not of the
/// lane.
#[tokio::test]
async fn a_read_only_type_composes_a_sandbox_nothing_can_write() {
    let sandbox = Sandbox::new("helper-mode-cell-cap-composition");
    let mode = shell_policy::ModeCell::new(ApprovalPolicy::Bypass);
    let Some(shell) = helper_shell_following_mode(&sandbox, &mode, ApprovalPolicy::ReadOnly, None)
    else {
        return;
    };

    let wrote = shell
        .run_call(&sandbox_call("touch created.txt"))
        .await
        .expect("the sandbox lane serves a read-only type");
    assert_ne!(
        wrote["exit_code"], 0,
        "nothing is writable for a read-only type: {wrote}"
    );
    assert!(!sandbox.ws_root().join("created.txt").exists());
}
