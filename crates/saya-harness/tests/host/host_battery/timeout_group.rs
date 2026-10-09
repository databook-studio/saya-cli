//! Timeout behavior: a timeout kills the whole process group including a
//! daemonizing grandchild, and a requested timeout above the configured
//! ceiling refuses with both numbers.

use std::{io, time::Duration};

use super::{config, leak, stage};

use saya_harness::host::{HostCommand, HostConfig, HostError};

/// `true` when no member of the child's process group remains: signal 0
/// to the group returns ESRCH exactly when everything died.
fn group_gone(pgid: u32) -> bool {
    let result = unsafe { libc::killpg(pgid as libc::pid_t, 0) };
    result == -1 && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
}

fn wait_group_gone(pgid: u32) -> bool {
    let start = std::time::Instant::now();
    loop {
        if group_gone(pgid) {
            return true;
        }
        if start.elapsed() > Duration::from_secs(10) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[tokio::test]
async fn timeout_kills_the_process_group_including_a_daemonizing_grandchild() {
    let dir = leak("daemon");
    stage(&dir, "daemon-probe");
    let config = HostConfig::new(
        dir.display().to_string(),
        std::env::temp_dir(),
        Duration::from_secs(2),
    )
    .expect("shaped");
    let outcome = HostCommand::new("daemon-probe", ["fork-daemon".to_owned()])
        .expect("shaped")
        .run(&config, None, &saya_agent::CancellationToken::new())
        .await
        .expect("the run itself must complete");
    assert!(outcome.killed_by_timeout, "the timeout must have fired");
    assert_eq!(outcome.exit_code, None, "a killed child has no exit code");
    assert!(
        wait_group_gone(outcome.pid),
        "the double-forked grandchild must be gone with the group"
    );
}

#[tokio::test]
async fn a_wider_timeout_than_the_ceiling_refuses_with_both_numbers() {
    let dir = leak("ceiling");
    stage(&dir, "ceiling-probe");
    let config = config(&dir.display().to_string(), &[]);
    let error = HostCommand::new("ceiling-probe", ["exit".to_owned(), "0".to_owned()])
        .expect("shaped")
        .run(&config, Some(601), &saya_agent::CancellationToken::new())
        .await
        .expect_err("a request above the ceiling is refused");
    assert!(
        matches!(
            &error,
            HostError::TimeoutExceedsCeiling {
                requested: 601,
                ceiling: 600
            }
        ),
        "the refusal must carry both numbers: {error}"
    );
    let outcome = HostCommand::new("ceiling-probe", ["exit".to_owned(), "0".to_owned()])
        .expect("shaped")
        .run(&config, Some(30), &saya_agent::CancellationToken::new())
        .await
        .expect("a request at or below the ceiling narrows");
    assert_eq!(outcome.exit_code, Some(0));
}
