//! The timeout bounds a call is validated against: the default a model call
//! gets when it states none, the widest timeout a call may state, and the
//! refusal above it. The numbers are the shell's documented bounds — 120 s
//! foreground, 600 s ceiling — so a runaway search returns in two minutes,
//! while a long test run can still ask for ten.

use std::time::Duration;

use crate::runner::RunnerError;
use crate::runner::shell::{
    DEFAULT_TIMEOUT, HostPolicy, MAX_COMMAND_BYTES, MAX_TIMEOUT, ShellCall, WorkspaceShell,
};

/// A shell over a real directory: validation resolves the root, so the root
/// must exist. The temp dir is never written.
fn shell() -> WorkspaceShell {
    WorkspaceShell::host_only(std::env::temp_dir(), HostPolicy::default())
        .expect("the temp dir resolves")
}

/// The timeout a call is validated with.
fn timeout_of(call: ShellCall) -> Duration {
    shell()
        .validated_call(&call)
        .expect("the call is admitted")
        .0
        .timeout
}

/// A model call that states no timeout gets the 120 s foreground default.
#[test]
fn a_call_without_a_timeout_gets_the_120_second_default() {
    assert_eq!(DEFAULT_TIMEOUT, Duration::from_secs(120));
    assert_eq!(timeout_of(ShellCall::new("true")), Duration::from_secs(120));
}

/// A call may widen the default up to the 600 s ceiling, so a long test run
/// can ask for the whole window.
#[test]
fn a_call_may_state_up_to_the_600_second_ceiling() {
    assert_eq!(MAX_TIMEOUT, Duration::from_secs(600));
    assert_eq!(
        timeout_of(ShellCall::new("true").with_timeout(Duration::from_secs(600))),
        Duration::from_secs(600)
    );
}

/// The command ceiling is 64 KiB, and a call one byte over is refused before
/// anything spawns with the call's size and the ceiling in the message.
#[test]
fn a_command_over_the_64_kib_ceiling_is_refused_with_its_size() {
    assert_eq!(MAX_COMMAND_BYTES, 64 * 1024);
    assert!(
        shell()
            .validated_call(&ShellCall::new("x".repeat(60 * 1024)))
            .is_ok(),
        "a 60 KiB command is admitted"
    );
    let over = MAX_COMMAND_BYTES + 1;
    let error = shell()
        .validated_call(&ShellCall::new("x".repeat(over)))
        .expect_err("the ceiling is a hard bound");
    let message = error.to_string();
    assert!(
        message.contains(&over.to_string()),
        "the refusal names the call's size: {message}"
    );
    assert!(
        message.contains(&MAX_COMMAND_BYTES.to_string()),
        "the refusal names the ceiling: {message}"
    );
}

/// Above the ceiling the call is refused before anything spawns, exactly as
/// before: the ceiling is a hard bound, never a silent clamp.
#[test]
fn a_call_above_the_ceiling_is_refused() {
    let error = shell()
        .validated_call(&ShellCall::new("true").with_timeout(Duration::from_millis(600_001)))
        .expect_err("the ceiling is a hard bound");
    assert!(
        matches!(error, RunnerError::ShellCommandRefused { .. }),
        "the refusal is typed: {error:?}"
    );
}
