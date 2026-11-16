//! The background read's silent-poll backoff: the first read of a run answers
//! at once, and a read that follows a silent read of the same live run waits
//! for a change — 30 s, then 60 s, then 120 s — clamped to the job's wall
//! clock. Any output or the exit resets the sequence, and cancellation
//! interrupts the wait.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::path::Path;
use std::time::{Duration, Instant};

use saya_agent::{ApprovalPolicy, CancellationToken};
use saya_harness::runner::HostPolicy;
use serde_json::{Value, json};

use super::root_shell::RootShell;
use super::root_shell_tests::fixture::TempCheckout;
use crate::agent::tools::shell_policy::{Background, WallClock, compose_host_only};

/// Short waits for the battery: the production table is 30/60/120 s, and a
/// test must never spend one of those.
const TEST_WAITS: [Duration; 3] = [
    Duration::from_millis(1200),
    Duration::from_secs(3),
    Duration::from_secs(3),
];

fn shell_over(
    checkout: &Path,
    cancellation: CancellationToken,
    wall_clock: Option<Duration>,
) -> Option<RootShell> {
    let tool = compose_host_only(
        checkout,
        ApprovalPolicy::Bypass,
        None,
        HostPolicy::default(),
        cancellation,
        Background::Owned,
        None,
        saya_config::CommandRules::default(),
    )?;
    let tool = tool.with_silent_waits(TEST_WAITS);
    let tool = match wall_clock {
        Some(limit) => tool.with_wall_clock(WallClock::new(limit)),
        None => tool,
    };
    Some(RootShell::new(tool))
}

async fn start(shell: &RootShell, command: &str) -> u64 {
    let started = shell
        .call(
            "shell",
            &json!({"command": command, "run_in_background": true, "sandbox": false}),
        )
        .await
        .expect("a background start runs");
    started["id"]
        .as_u64()
        .expect("the start answers with an id")
}

async fn read(shell: &RootShell, id: u64) -> Value {
    shell
        .call("shell_output", &json!({"id": id}))
        .await
        .expect("the run is readable")
}

/// The first read of a live id answers at once; the next one — the previous
/// read was silent — waits the first backoff.
#[tokio::test]
async fn a_second_silent_read_waits_for_the_first_backoff() {
    let checkout = TempCheckout::new("silent-wait");
    let Some(shell) = shell_over(checkout.path(), CancellationToken::new(), None) else {
        return;
    };
    let id = start(&shell, "sleep 30").await;

    let began = Instant::now();
    let first = read(&shell, id).await;
    assert!(
        began.elapsed() < TEST_WAITS[0],
        "the first read answers at once: {:?}",
        began.elapsed()
    );
    assert_eq!(first["status"], "running", "{first}");

    let began = Instant::now();
    let second = read(&shell, id).await;
    let waited = began.elapsed();
    assert!(
        waited >= TEST_WAITS[0].saturating_sub(Duration::from_millis(100)),
        "the second silent read waits the first backoff: {waited:?}"
    );
    assert!(
        waited < Duration::from_secs(2),
        "the wait is the first backoff, not longer: {waited:?}"
    );
    assert_eq!(second["status"], "running", "{second}");
    let _ = shell.call("shell_kill", &json!({"id": id})).await;
}

/// New output ends the wait and resets the sequence: the read that sees it
/// returns at the write, and the next read answers at once again.
#[tokio::test]
async fn new_output_resets_the_silent_sequence() {
    let checkout = TempCheckout::new("silent-reset");
    let Some(shell) = shell_over(checkout.path(), CancellationToken::new(), None) else {
        return;
    };
    // Writes once, then stays live, so the write is the only change. The
    // write lands inside the first backoff the second read is allowed.
    let id = start(&shell, "sleep 0.5; echo late; sleep 30").await;
    let first = read(&shell, id).await;
    assert_eq!(first["status"], "running", "{first}");
    assert_eq!(first["stdout"], "", "nothing has been written yet: {first}");

    let began = Instant::now();
    let second = read(&shell, id).await;
    let waited = began.elapsed();
    assert_eq!(
        second["stdout"], "late\n",
        "the read returns the write: {second}"
    );
    assert_eq!(second["status"], "running", "{second}");
    assert!(
        waited < TEST_WAITS[1],
        "the read returns at the write, not at the backoff: {waited:?}"
    );

    // The write reset the sequence: this read is a first read again.
    let began = Instant::now();
    let third = read(&shell, id).await;
    assert!(
        began.elapsed() < TEST_WAITS[0],
        "a write resets the silent sequence: {:?}",
        began.elapsed()
    );
    assert_eq!(third["status"], "running", "{third}");
    let _ = shell.call("shell_kill", &json!({"id": id})).await;
}

/// A cancellation interrupts the wait at once: the read reports the cancelled
/// run rather than holding the call for the backoff.
#[tokio::test]
async fn cancellation_interrupts_a_waiting_read() {
    let checkout = TempCheckout::new("silent-cancel");
    let cancellation = CancellationToken::new();
    let Some(shell) = shell_over(checkout.path(), cancellation.clone(), None) else {
        return;
    };
    let id = start(&shell, "sleep 30").await;
    let first = read(&shell, id).await;
    assert_eq!(first["status"], "running", "{first}");

    let began = Instant::now();
    let arguments = json!({"id": id});
    let mut waiting = Box::pin(shell.call("shell_output", &arguments));
    tokio::select! {
        _ = tokio::time::sleep(Duration::from_millis(200)) => cancellation.cancel(),
        _ = &mut waiting => panic!("the second read must be waiting"),
    }
    let value = tokio::time::timeout(Duration::from_secs(2), &mut waiting)
        .await
        .expect("the cancellation interrupts the wait")
        .expect("the read reports the cancelled run");
    assert_eq!(value["status"], "cancelled", "{value}");
    assert!(
        began.elapsed() < Duration::from_secs(2),
        "the cancellation ends the wait at once: {:?}",
        began.elapsed()
    );
}

/// The job's wall clock clamps a backoff that would outlast it.
#[tokio::test]
async fn a_wait_is_clamped_to_the_job_wall_clock() {
    let checkout = TempCheckout::new("silent-clock");
    let Some(shell) = shell_over(
        checkout.path(),
        CancellationToken::new(),
        Some(Duration::from_millis(600)),
    ) else {
        return;
    };
    let id = start(&shell, "sleep 30").await;
    let first = read(&shell, id).await;
    assert_eq!(first["status"], "running", "{first}");

    let began = Instant::now();
    let second = read(&shell, id).await;
    let waited = began.elapsed();
    assert_eq!(second["status"], "running", "{second}");
    assert!(
        waited >= Duration::from_millis(250),
        "the read still waits what the clock allows: {waited:?}"
    );
    assert!(
        waited < Duration::from_secs(2),
        "the job's clock clamps the backoff: {waited:?}"
    );
    let _ = shell.call("shell_kill", &json!({"id": id})).await;
}
