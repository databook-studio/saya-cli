//! Tests 4, 5, 6 and 9: how a run ends. Two are the bounded-work guarantees
//! (the ring cap, the group kill on timeout and on cancellation); the last is
//! the call's own bounds, refused before anything spawns.

use std::time::{Duration, Instant};

use saya_agent::{CancellationReason, CancellationToken};

use super::{alive, proven_shell};
use crate::runner::shell::MAX_COMMAND_BYTES;
use crate::runner::{OUTPUT_CAP_BYTES, RunnerError, ShellCall};

/// Twenty megabytes through a sixty-four kilobyte ring: the capture stays
/// inside the cap, keeps the first line as well as the last, and the drop is
/// reported rather than hidden.
#[tokio::test]
async fn a_runaway_stream_is_capped_and_reported_truncated() {
    let Some(proven) = proven_shell("truncate") else {
        return;
    };
    let outcome = proven
        .shell
        .run(ShellCall::new(
            "printf 'HEAD\\n'; yes . | head -c 20000000; printf '\\nTAIL\\n'",
        ))
        .await
        .expect("the shell must run");
    let text = &outcome.stdout.text;
    assert!(
        outcome.truncated,
        "twenty megabytes through a {OUTPUT_CAP_BYTES} byte cap must report truncation"
    );
    assert!(
        text.len() <= OUTPUT_CAP_BYTES,
        "the capture must stay inside the cap: {} bytes",
        text.len()
    );
    assert!(
        outcome.stdout.total_bytes > OUTPUT_CAP_BYTES as u64,
        "the child really did write more than the cap: {} bytes",
        outcome.stdout.total_bytes
    );
    assert!(text.starts_with("HEAD\n"), "the start of the run survives");
    assert!(text.ends_with("\nTAIL\n"), "the end of the run survives");
    assert!(
        text.contains(&format!(
            "\n... [{} bytes elided] ...\n",
            outcome.stdout.dropped_bytes
        )),
        "the elided middle is marked with the exact count: {} bytes",
        outcome.stdout.dropped_bytes
    );
}

/// The timeout kills the child's whole process group, and nothing survives
/// it: a background job is exactly the process a single-child kill misses.
#[tokio::test]
async fn a_timeout_kills_the_process_group_and_leaves_nothing_alive() {
    let Some(proven) = proven_shell("timeout") else {
        return;
    };
    // A unique marker, so the liveness probe cannot match another test's
    // child or a stale process from an earlier run.
    let marker = format!("sleep 600.{:04}", std::process::id() % 10_000);
    let started = Instant::now();
    let outcome = proven
        .shell
        .run(ShellCall::new(format!("{marker} & {marker}")).with_timeout(Duration::from_secs(1)))
        .await
        .expect("the shell must run");
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(5),
        "the timeout must fire well inside five seconds; took {elapsed:?}"
    );
    assert!(outcome.started, "the child did start");
    assert!(outcome.timed_out, "the timeout must be reported");
    assert!(
        !alive(&marker),
        "no process from the killed group may survive the run"
    );
}

/// Cancellation reaches the same group kill, and a run that started is
/// reported as started — never as "no effect".
#[tokio::test]
async fn cancellation_stops_a_running_shell_and_reports_it() {
    let Some(proven) = proven_shell("cancel") else {
        return;
    };
    let cancellation = CancellationToken::new();
    let shell = proven.shell.with_cancellation(cancellation.clone());
    let fired = cancellation.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        fired.cancel_with_reason(CancellationReason::User);
    });

    let started = Instant::now();
    let outcome = shell
        .run(ShellCall::new("sleep 600"))
        .await
        .expect("the shell must run");
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(5),
        "cancellation must land well inside five seconds; took {elapsed:?}"
    );
    assert!(outcome.started, "the child did start");
    assert!(outcome.cancelled, "the cancellation must be reported");
    assert!(!outcome.timed_out, "the timeout did not fire");
}

/// The call's bounds are typed refusals, earned before anything spawns.
#[tokio::test]
async fn the_calls_bounds_are_refused_before_anything_spawns() {
    let Some(proven) = proven_shell("bounds") else {
        return;
    };
    let cases = [
        ShellCall::new("a".repeat(MAX_COMMAND_BYTES + 1)),
        ShellCall::new("true").with_timeout(Duration::ZERO),
        ShellCall::new("true").with_timeout(Duration::from_millis(600_001)),
        ShellCall::new("true\0exit 0"),
    ];
    for call in cases {
        let error = proven
            .shell
            .run(call)
            .await
            .expect_err("the bound must be refused");
        assert!(
            matches!(error, RunnerError::ShellCommandRefused { .. }),
            "the refusal must be typed: {error:?}"
        );
    }
}
