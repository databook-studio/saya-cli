//! The root shell's battery: the root advertises `shell`
//! whenever the config is on and the mode is not `never`; a call that names
//! no sandbox is a host call, asked of the operator under `ask` and allowed
//! outright under `bypass`; a call that names the sandbox runs there without
//! asking; and an approved call may write `.git` in either lane. The headless
//! `ask` lane's own battery — what a call with no operator to answer does, and
//! what its result says — is `notice`.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::fs;
use std::sync::Arc;

use saya_agent::{ApprovalDecider, ApprovalPolicy};
use saya_harness::runner::HostPolicy;

use super::root_shell;
use crate::agent::tools::child::tests_root_compose_fixture as compose_fixture;
use crate::agent::tools::shell_policy::definition;

#[path = "root_shell_fixture.rs"]
pub(super) mod fixture;
#[path = "root_shell_host_tests.rs"]
mod host;
#[path = "root_shell_notice_tests.rs"]
mod notice;

use fixture::{
    ApproveAll, READ_ONLY_REFUSAL, RecordingOperator, TempCheckout, call, host_call,
    host_shell_over, sandbox_call, shell_over,
};

/// (a) Under `ask` with an operator present, a call that names no sandbox is a
/// host call: the operator is asked exactly once, and the answer decides.
#[tokio::test]
async fn ask_puts_a_call_with_no_sandbox_to_the_operator_once() {
    let checkout = TempCheckout::new("host-ask");
    let approving = RecordingOperator::new(true);
    let Some(shell) = shell_over(
        checkout.path(),
        ApprovalPolicy::Ask,
        Some(approving.clone()),
    ) else {
        return;
    };
    assert!(shell.approve(&definition(), &call("true")).await);
    assert_eq!(approving.calls(), 1, "a host call asks exactly once");
    assert!(shell.refusal_detail(&definition(), &call("true")).is_none());
    let ran = shell
        .call("shell", &call("true"))
        .await
        .expect("the approved host call runs");
    assert_eq!(ran["sandboxed"], false, "the call ran on the host: {ran}");

    let refusing = RecordingOperator::new(false);
    let Some(shell) = shell_over(checkout.path(), ApprovalPolicy::Ask, Some(refusing.clone()))
    else {
        return;
    };
    assert!(!shell.approve(&definition(), &host_call("true")).await);
    assert_eq!(refusing.calls(), 1);
}

/// (b) A call that names the sandbox runs there with no operator call at all —
/// with one attached and with none.
#[tokio::test]
async fn a_sandbox_call_runs_without_asking_the_operator() {
    let checkout = TempCheckout::new("sandboxed-ask");
    let operator = RecordingOperator::new(false);
    let Some(shell) = shell_over(checkout.path(), ApprovalPolicy::Ask, Some(operator.clone()))
    else {
        return;
    };
    assert!(shell.approve(&definition(), &sandbox_call("true")).await);
    assert_eq!(
        operator.calls(),
        0,
        "the confinement is the consent; no operator is asked"
    );
    assert!(
        shell
            .refusal_detail(&definition(), &sandbox_call("true"))
            .is_none()
    );
    let ran = shell
        .call("shell", &sandbox_call("true"))
        .await
        .expect("the sandboxed call runs");
    assert_eq!(ran["sandboxed"], true, "the call ran in the sandbox: {ran}");

    // The operator is optional: without one the sandboxed call is still allowed.
    let Some(shell) = shell_over(checkout.path(), ApprovalPolicy::Ask, None) else {
        return;
    };
    assert!(shell.approve(&definition(), &sandbox_call("true")).await);
}

/// (e) `bypass` runs a call that names no sandbox on the host and never asks.
#[tokio::test]
async fn bypass_runs_a_call_with_no_sandbox_on_the_host_without_asking() {
    let checkout = TempCheckout::new("host-bypass");
    let operator = RecordingOperator::new(false);
    let Some(shell) = shell_over(
        checkout.path(),
        ApprovalPolicy::Bypass,
        Some(operator.clone()),
    ) else {
        return;
    };
    assert!(shell.approve(&definition(), &call("true")).await);
    assert_eq!(operator.calls(), 0, "bypass never asks");
    assert!(shell.refusal_detail(&definition(), &call("true")).is_none());
    let ran = shell
        .call("shell", &call("true"))
        .await
        .expect("the host call runs");
    assert_eq!(ran["sandboxed"], false, "bypass runs on the host: {ran}");
}

/// (f) `.git` is not write-protected: the same command may create a file
/// under it in the sandbox and on the host.
#[tokio::test]
async fn a_command_may_write_git_in_both_lanes() {
    let checkout = TempCheckout::new("git-write");

    let Some(shell) = shell_over(checkout.path(), ApprovalPolicy::Ask, None) else {
        return;
    };
    let sandboxed = shell
        .call("shell", &sandbox_call("mkdir -p .git/x && touch .git/x/y"))
        .await
        .expect("a sandboxed call runs");
    assert_eq!(
        sandboxed["exit_code"], 0,
        "the sandbox may write .git: {sandboxed}"
    );
    assert_eq!(sandboxed["sandboxed"], true);
    assert!(checkout.path().join(".git/x/y").exists());

    let Some(shell) = shell_over(checkout.path(), ApprovalPolicy::Bypass, None) else {
        return;
    };
    let host = shell
        .call("shell", &call("mkdir -p .git/z && touch .git/z/w"))
        .await
        .expect("a host call runs");
    assert_eq!(host["exit_code"], 0, "the host may write .git: {host}");
    assert_eq!(host["sandboxed"], false);
    assert!(checkout.path().join(".git/z/w").exists());
}

/// `read-only` serves a host call from the sandbox it keeps, where the whole
/// checkout is write-denied: the call still reads and runs, and changes
/// nothing.
#[tokio::test]
async fn read_only_serves_a_host_call_from_the_frozen_sandbox() {
    let checkout = TempCheckout::new("read-only");
    fs::write(checkout.path().join("existing.txt"), "readable\n").unwrap();
    let Some(shell) = shell_over(
        checkout.path(),
        ApprovalPolicy::ReadOnly,
        Some(Arc::new(ApproveAll)),
    ) else {
        eprintln!("skipping `read-only`: the write-denied root does not prove on this host");
        return;
    };
    assert!(
        shell.approve(&definition(), &host_call("true")).await,
        "a host call is served from the sandbox, not refused"
    );
    assert!(
        shell
            .refusal_detail(&definition(), &host_call("true"))
            .is_none()
    );

    let wrote = shell
        .call("shell", &host_call("touch created.txt"))
        .await
        .expect("the call runs in the sandbox");
    assert_ne!(wrote["exit_code"], 0, "nothing is writable: {wrote}");
    assert_eq!(wrote["sandboxed"], true, "the host call ran sandboxed");
    assert!(!checkout.path().join("created.txt").exists());

    let read = shell
        .call("shell", &call("cat existing.txt"))
        .await
        .expect("reading is still allowed");
    assert_eq!(read["exit_code"], 0, "{read}");
    assert!(
        read["stdout"]
            .as_str()
            .unwrap_or_default()
            .contains("readable")
    );
    assert_eq!(read["sandboxed"], true);
}

/// `read-only` on a host with no proven sandbox has no lane to serve a call
/// from, so it refuses outright rather than reaching the host.
#[tokio::test]
async fn read_only_refuses_when_no_sandbox_is_proven() {
    let checkout = TempCheckout::new("read-only-unproven");
    let Some(shell) = host_shell_over(
        checkout.path(),
        ApprovalPolicy::ReadOnly,
        None,
        HostPolicy::default(),
    ) else {
        return;
    };
    assert!(!shell.approve(&definition(), &call("true")).await);
    assert_eq!(
        shell
            .refusal_detail(&definition(), &call("true"))
            .as_deref(),
        Some(READ_ONLY_REFUSAL)
    );
}

/// No shell under `never`, and one in every mode that may run a command:
/// `[tools] allow` is the one switch that removes the shell, and it narrows
/// after composition.
#[tokio::test]
async fn the_shell_is_advertised_unless_the_mode_forbids_it() {
    assert!(
        shell_over(
            TempCheckout::new("never").path(),
            ApprovalPolicy::Never,
            None
        )
        .is_none(),
        "`never` advertises no shell at all"
    );

    let fixture = compose_fixture::Fixture::new("root-shell-absent", &["tracked.txt"]).await;
    let root = fixture.checkout_path.as_path();
    if shell_over(root, ApprovalPolicy::Ask, None).is_some() {
        assert!(
            root_shell::build(
                &fixture.delegate_job,
                root,
                std::sync::Arc::new(crate::agent::tools::workspace::ReadWitnesses::default()),
                saya_harness::workspace::Workspace::open(root)
                    .ok()
                    .map(std::sync::Arc::new),
            )
            .is_some(),
            "a mode that may run a command advertises the shell"
        );
    }
}

/// An approved sandboxed call writes inside the checkout and reports the
/// fields a call needs: the exit code, the two streams, and the lane it took.
/// The flags that sit at their default are absent — `truncated` and
/// `timed_out` are served only when true, and a cancelled run is an error
/// rather than a result, so `cancelled` is never carried at all.
#[tokio::test]
async fn an_approved_call_writes_inside_the_checkout() {
    let checkout = TempCheckout::new("writes");
    let Some(shell) = shell_over(checkout.path(), ApprovalPolicy::Ask, None) else {
        return;
    };
    let wrote = shell
        .call("shell", &sandbox_call("echo hi > f.txt"))
        .await
        .expect("an approved write runs");
    assert_eq!(wrote["exit_code"], 0, "{wrote}");
    assert_eq!(
        fs::read_to_string(checkout.path().join("f.txt")).unwrap(),
        "hi\n",
        "the write lands in the working checkout"
    );
    let object = wrote.as_object().expect("the result is a JSON object");
    for field in ["exit_code", "stdout", "stderr", "sandboxed"] {
        assert!(object.contains_key(field), "{field} must be reported");
    }
    for absent in ["truncated", "timed_out", "cancelled"] {
        assert!(
            !object.contains_key(absent),
            "{absent} sits at its default and must not be reported: {wrote}"
        );
    }
    assert_eq!(wrote["sandboxed"], true);
}
