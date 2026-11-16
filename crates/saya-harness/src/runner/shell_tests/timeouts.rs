//! Test 25: the foreground timeout's handoff. A call that hits its timeout is
//! moved into the background registry instead of killed, and the id it answers
//! with reads the still-running command back. The timeout here is short: the
//! handoff path does not branch on which default produced it, and the 120 s
//! default itself is pinned in `shell::validate_tests`.

use std::time::{Duration, Instant};

use crate::runner::{RunStatus, ShellCall};

use super::proven_shell;

#[tokio::test]
async fn a_foreground_call_that_hits_its_timeout_moves_to_the_background() {
    let Some(proven) = proven_shell("detach-timeout") else {
        return;
    };
    let shell = proven.shell.with_timeout_detach(true);
    let marker = format!("sleep 30.{:04}", std::process::id() % 10_000);
    let started = Instant::now();
    let outcome = shell
        .run(ShellCall::new(marker).with_timeout(Duration::from_millis(100)))
        .await
        .expect("the shell must run");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the timeout must fire well inside five seconds; took {:?}",
        started.elapsed()
    );
    assert!(outcome.timed_out, "the move is reported as a timeout");
    let id = outcome
        .moved_to_background
        .expect("the run moved to the background");
    let report = shell
        .kill_background(id)
        .await
        .expect("the moved run is readable by its id");
    assert!(
        matches!(report.status, RunStatus::Killed),
        "the kill ends the moved run: {:?}",
        report.status
    );
}
