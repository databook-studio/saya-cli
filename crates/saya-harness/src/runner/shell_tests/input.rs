//! The call's stdin: 25–27. A call may carry a bounded input the child reads
//! on stdin; a call that carries none leaves the child's stdin at EOF, exactly
//! as every call did before this existed.
//!
//! These run on a **host-only** shell, which needs no sandbox, so none of them
//! skips on an unproven host.
//!
//! 25. a call's input reaches the child on stdin, byte for byte, and the pipe
//!     is closed afterwards so the child sees EOF;
//! 26. a call with no input reads EOF immediately rather than waiting;
//! 27. an input over the bound is refused before anything spawns.

use std::time::Duration;

use super::host::host_only_shell;
use crate::runner::shell::MAX_STDIN_BYTES;
use crate::runner::{HostPolicy, RunnerError, ShellCall};

/// The child echoes its whole stdin back: what the caller wrote is what the
/// child read, and the read ends because the pipe closed, not because the
/// child gave up waiting.
#[tokio::test]
async fn a_calls_input_reaches_the_child_and_then_ends() {
    let host = host_only_shell("stdin-payload", HostPolicy::default());
    let outcome = host
        .shell
        .run(
            ShellCall::new("cat")
                .with_sandbox(false)
                .with_input(b"{\"hook_event_name\":\"Stop\"}"),
        )
        .await
        .expect("the host run must run");
    assert_eq!(
        outcome.exit_code,
        Some(0),
        "cat must read to EOF and exit cleanly: {:?}",
        outcome.stderr.text
    );
    assert_eq!(outcome.stdout.text, "{\"hook_event_name\":\"Stop\"}");
}

/// A call that carries no input leaves the child's stdin closed: a reader
/// returns at once with nothing rather than blocking until the timeout.
#[tokio::test]
async fn a_call_without_input_reads_eof_at_once() {
    let host = host_only_shell("stdin-empty", HostPolicy::default());
    let outcome = host
        .shell
        .run(
            ShellCall::new("cat")
                .with_sandbox(false)
                .with_timeout(Duration::from_secs(5)),
        )
        .await
        .expect("the host run must run");
    assert_eq!(outcome.exit_code, Some(0), "cat must not be killed");
    assert!(!outcome.timed_out, "an empty stdin is EOF, not a wait");
    assert_eq!(outcome.stdout.text, "");
}

/// The input bound is checked where every other bound is: before the spawn, as
/// a typed refusal. Nothing runs.
#[tokio::test]
async fn an_oversized_input_is_refused_before_the_spawn() {
    let host = host_only_shell("stdin-oversized", HostPolicy::default());
    let oversized = vec![b'x'; MAX_STDIN_BYTES + 1];
    let error = host
        .shell
        .run(
            ShellCall::new("touch landed")
                .with_sandbox(false)
                .with_input(oversized.clone()),
        )
        .await
        .expect_err("an over-bound input must be refused");
    assert!(
        matches!(error, RunnerError::ShellCommandRefused { .. }),
        "the refusal is the shell's own typed one: {error:?}"
    );
    assert!(
        !host.root.join("landed").exists(),
        "nothing may run after a refusal"
    );
}
