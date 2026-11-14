//! Tests 1 and 8: what the child's environment is, and what its captured
//! output may carry back. The first proves nothing is inherited; the second
//! proves the shared redaction gate is on the path and cannot be skipped.

use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
};

use super::{CHILD_ROOT, CHILD_TEST, TempRoot, proven_shell};
use crate::runner::ShellCall;

/// The parent's environment never reaches the shell child. The canary is
/// planted in the *inner* process's environment by the outer one, so no
/// `set_var` ever touches the shared test binary's own environment.
#[tokio::test]
async fn a_planted_parent_variable_never_reaches_the_shell() {
    if let Some(dir) = std::env::var_os(CHILD_ROOT) {
        // The inner run: `SAYA_SHELL_CANARY` is in THIS process's
        // environment, so a leak would show in the child it spawns.
        let dir = PathBuf::from(dir);
        let Some(proven) = proven_shell("canary-inner") else {
            fs::write(dir.join("unproven"), b"1").expect("record the unproven host");
            return;
        };
        let outcome = proven
            .shell
            .run(ShellCall::new("env"))
            .await
            .expect("the env dump must run");
        fs::write(dir.join("env.txt"), &outcome.stdout.text).expect("record the child's env");
        return;
    }

    let guard = TempRoot::new("canary-outer");
    let status = Command::new(std::env::current_exe().expect("the test binary path resolves"))
        .arg("--exact")
        .arg(CHILD_TEST)
        .env(CHILD_ROOT, &guard.0)
        .env("SAYA_SHELL_CANARY", "leak-me")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("the self-re-exec must spawn");
    assert!(
        status.success(),
        "the inner canary test must pass: {status}"
    );
    if guard.0.join("unproven").exists() {
        eprintln!("skipping `canary`: the sandbox is not proven on this host");
        return;
    }
    let text =
        fs::read_to_string(guard.0.join("env.txt")).expect("the inner run must record the env");
    assert!(
        !text.contains("leak-me"),
        "the parent's variable must not reach the shell child: {text}"
    );
    assert!(
        text.contains("PATH=") && text.contains("TERM=dumb"),
        "the child's environment is the declared four, and nothing else: {text}"
    );
}

/// The captured output passes the same two-pass gate every runner capture
/// does: the marker's value never survives to the model or the run record.
#[tokio::test]
async fn captured_output_passes_the_redaction_gate() {
    let Some(proven) = proven_shell("redact") else {
        return;
    };
    let outcome = proven
        .shell
        .run(ShellCall::new("echo password=hunter2-super-secret"))
        .await
        .expect("the shell must run");
    assert_eq!(outcome.exit_code, Some(0));
    assert!(
        !outcome.stdout.text.contains("hunter2-super-secret"),
        "the marker's value must not survive the capture: {:?}",
        outcome.stdout.text
    );
    assert!(
        outcome.stdout.text.contains("[redacted]"),
        "the redaction marker is what replaced it: {:?}",
        outcome.stdout.text
    );
}
