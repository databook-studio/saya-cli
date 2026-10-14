use std::{env, fs, num::NonZeroU64, path::PathBuf, time::Duration};

use saya_harness::coding::commands::{CommandOutputError, CommandOwner};

use crate::support::{Fixture, HELPER, create_private_dir, fixture, run_test_scenario_in_child};

#[path = "store/flood.rs"]
mod flood;
#[path = "store/redaction.rs"]
mod redaction;
#[path = "store/start.rs"]
mod start;

/// Env var selecting the isolated file-failure scenario in the re-exec'd
/// child, carrying the completion marker path the parent checks. The exact
/// child test name keeps the process-wide `RLIMIT_FSIZE`/`SIGXFSZ` changes
/// confined to the child process.
const FILE_FAIL_CHILD_ENV: &str = "SAYA_C02_STORE_FILE_FAIL_DONE";
const FILE_FAIL_CHILD_TEST: &str =
    "output_capture::store::flood_with_unwritable_store_drains_child_and_reports_store_error";

/// A mid-stream store write failure never stops the child: the writer keeps
/// draining its channel without writing — the drained bytes count as
/// dropped in the store status — the child still exits 0 with its
/// `ProgramOutcome` fully intact, and the typed store status reports the
/// failure without message text. A post-creation `chmod` cannot fail an
/// already-open descriptor, so the honest lever is `RLIMIT_FSIZE` lowered
/// after creation with `SIGXFSZ` ignored — process-wide state a parallel
/// test binary must not touch, so the scenario runs only in a re-exec'd
/// child selected by `FILE_FAIL_CHILD_ENV`.
#[tokio::test]
async fn flood_with_unwritable_store_drains_child_and_reports_store_error() {
    let Some(fixture) = fixture() else { return };
    if let Some(done) = env::var_os(FILE_FAIL_CHILD_ENV) {
        run_file_fail_scenario(fixture, PathBuf::from(done)).await;
        return;
    }
    let done_parent = fixture.marker("store-file-fail");
    // The child writes its completion marker here, so the parent exists.
    create_private_dir(&done_parent);
    let done = done_parent.join("child-complete");
    run_test_scenario_in_child(FILE_FAIL_CHILD_ENV, FILE_FAIL_CHILD_TEST, &done);
}

/// The file-failure scenario itself, run only in the re-exec'd child.
async fn run_file_fail_scenario(fixture: &Fixture, done: PathBuf) {
    let owner = CommandOwner::new(1).unwrap();
    let session = owner
        .start_with_output(
            fixture.run_program(),
            HELPER.to_owned(),
            vec!["drip".into(), "600".into()],
            NonZeroU64::new(30).unwrap(),
            fixture.marker("store-unwritable"),
        )
        .unwrap();
    // The store file exists and is open: lower the soft file-size limit so
    // its next append fails as the write's error rather than a signal death.
    let mut limit: libc::rlimit = unsafe { std::mem::zeroed() };
    let read_limit = unsafe { libc::getrlimit(libc::RLIMIT_FSIZE, &mut limit) };
    assert_eq!(read_limit, 0);
    let lowered = libc::rlimit {
        rlim_cur: limit.rlim_cur.min(4096),
        rlim_max: limit.rlim_max,
    };
    unsafe { libc::signal(libc::SIGXFSZ, libc::SIG_IGN) };
    assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_FSIZE, &lowered) }, 0);
    let drip_total: u64 = (0..600)
        .map(|i| format!("c02-drip {i}\n").len() as u64)
        .sum();
    // The store finishes as the run's last act, before the session's join
    // consumes it: the status is read before the join, per the contract.
    let status = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if let Some(ready) = session.output_status() {
                break ready;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the store reports its status once the run finished");
    let outcome = tokio::time::timeout(Duration::from_secs(30), session.join())
        .await
        .expect("the drip run settles within 30 s")
        .unwrap();
    assert_eq!(outcome.exit_code, Some(0), "the child still exits cleanly");
    assert!(!outcome.cancelled && !outcome.killed_by_timeout && outcome.signal.is_none());
    assert_eq!(outcome.stdout.total_bytes, drip_total);
    assert_eq!(outcome.stdout.dropped_bytes, 0, "the outcome is intact");
    assert!(
        matches!(
            status.error.as_ref(),
            Some(CommandOutputError::Store { .. })
        ),
        "the writer failure is typed without message text: {status:?}"
    );
    assert!(!status.capped, "a store failure is not a cap truncation");
    assert!(
        status.dropped_bytes > 0,
        "raw bytes the failed writer drained and discarded count as dropped: {status:?}"
    );
    // No handed count is exposed: raw total covers every stored byte.
    assert!(
        status.raw_total_bytes >= status.stored_bytes,
        "raw total covers every stored byte: {status:?}"
    );
    assert_eq!(status.raw_total_bytes, drip_total);
    assert!(
        status.stored_bytes <= lowered.rlim_cur,
        "the stored head stops at the failure point: {}",
        status.stored_bytes
    );
    fs::write(done, b"ok").expect("the scenario records its completion marker");
}
