//! The root shell's completion path: an explicit background start answers at
//! once, the fifth live run is refused by name, a run's exit is reported once
//! as a notice without its output, and `shell_output` answers a live run at
//! once instead of blocking inside the tool.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::time::{Duration, Instant};

use saya_agent::{ApprovalPolicy, BackgroundExit};
use serde_json::json;

use super::root_shell::RootShell;
use super::root_shell_tests::fixture::{TempCheckout, shell_over};

/// Starts `command` on the host lane and returns its id.
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

/// Polls the shell for exits until `count` have been reported or the deadline
/// passes. The registry notices an exit when the loop asks, not before.
async fn exits(shell: &RootShell, count: usize) -> Vec<BackgroundExit> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut found = Vec::new();
    while found.len() < count {
        found.extend(shell.take_background_exits());
        assert!(Instant::now() < deadline, "the runs did not exit in time");
        if found.len() < count {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    found
}

/// An explicit start returns its id before the command ends: the start does
/// not wait for the child.
#[tokio::test]
async fn a_background_start_returns_an_id_at_once() {
    let checkout = TempCheckout::new("exit-start");
    let Some(shell) = shell_over(checkout.path(), ApprovalPolicy::Bypass, None) else {
        return;
    };
    let began = Instant::now();
    let started = shell
        .call(
            "shell",
            &json!({"command": "sleep 5", "run_in_background": true, "sandbox": false}),
        )
        .await
        .expect("a background start runs");
    let elapsed = began.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "the start must not wait for the run: {elapsed:?}"
    );
    assert_eq!(started["status"], "running", "{started}");
    let id = started["id"].as_u64().expect("an id");
    let _ = shell.call("shell_kill", &json!({"id": id})).await;
}

/// The fifth live run is refused, and the refusal names `shell_kill` so the
/// model knows the way out.
#[tokio::test]
async fn the_fifth_live_run_is_refused_and_the_message_names_shell_kill() {
    let checkout = TempCheckout::new("exit-cap");
    let Some(shell) = shell_over(checkout.path(), ApprovalPolicy::Bypass, None) else {
        return;
    };
    let mut held = Vec::new();
    for _ in 0..saya_harness::runner::MAX_BACKGROUND_RUNS {
        held.push(start(&shell, "sleep 30").await);
    }
    let refused = shell
        .call(
            "shell",
            &json!({"command": "sleep 30", "run_in_background": true, "sandbox": false}),
        )
        .await
        .expect_err("the run past the cap is refused");
    let message = refused.to_string();
    assert!(
        message.contains("shell_kill") && message.contains("4 background runs"),
        "the refusal names the cap and the way out: {message}"
    );
    for id in held {
        let _ = shell.call("shell_kill", &json!({"id": id})).await;
    }
}

/// An exit is reported exactly once, with no command output, and the run stays
/// readable afterwards so the notice's `shell_output` instruction is true.
#[tokio::test]
async fn an_exit_produces_exactly_one_notice_and_stays_readable() {
    let checkout = TempCheckout::new("exit-notice");
    let Some(shell) = shell_over(checkout.path(), ApprovalPolicy::Bypass, None) else {
        return;
    };
    let first = start(&shell, "printf 'a\\nb\\n'").await;
    let second = start(&shell, "printf 'x\\n'").await;

    let found = exits(&shell, 2).await;
    assert_eq!(found.len(), 2, "each exit is reported once");
    let by_id = |id: u64| {
        found
            .iter()
            .find(|exit| exit.id == id)
            .unwrap_or_else(|| panic!("the exit of run {id} was reported: {found:?}"))
            .clone()
    };
    let first_exit = by_id(first);
    assert_eq!(first_exit.exit_code, Some(0), "{first_exit:?}");
    assert!(!first_exit.killed, "the command ended on its own");
    assert_eq!(first_exit.output_lines, 2, "two newline-terminated lines");
    let second_exit = by_id(second);
    assert_eq!(second_exit.exit_code, Some(0));
    assert_eq!(second_exit.output_lines, 1);
    assert!(
        shell.take_background_exits().is_empty(),
        "a taken exit is never reported again"
    );

    // The record is still there: the notice told the model to read it.
    let read = shell
        .call("shell_output", &json!({"id": first}))
        .await
        .expect("the run is still readable after its notice");
    assert_eq!(read["status"], "exited", "{read}");
    assert_eq!(read["stdout"], "a\nb\n", "{read}");
}

/// A read of a live run answers at once with its current tail and says the
/// loop will tell the model when it exits: no wait happens inside the tool.
#[tokio::test]
async fn a_live_run_answers_a_read_at_once_with_the_waiting_note() {
    let checkout = TempCheckout::new("exit-live-read");
    let Some(shell) = shell_over(checkout.path(), ApprovalPolicy::Bypass, None) else {
        return;
    };
    let id = start(&shell, "printf 'ready\\n'; sleep 30").await;
    // Let the first line reach the ring; the read itself must not wait.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let began = Instant::now();
    let read = shell
        .call("shell_output", &json!({"id": id}))
        .await
        .expect("the live run is readable");
    let elapsed = began.elapsed();
    assert!(
        elapsed < Duration::from_secs(1),
        "the read returns at once: {elapsed:?}"
    );
    assert_eq!(read["status"], "running", "{read}");
    assert_eq!(
        read["note"], "still running; you will be told when it exits",
        "the read says the loop will report the exit: {read}"
    );
    assert_eq!(read["stdout"], "ready\n", "{read}");
    let _ = shell.call("shell_kill", &json!({"id": id})).await;
}
