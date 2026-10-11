use std::{
    env, fs,
    num::NonZeroU64,
    path::PathBuf,
    time::{Duration, Instant},
};

use saya_harness::coding::commands::{
    CommandOutputError, CommandOwner, CommandStartError, MAX_OUTPUT_READ_BYTES,
    OUTPUT_STORE_CAP_BYTES, OutputStream, TRUNCATION_MARKER,
};

use crate::support::{
    CREDENTIAL_ENV, CREDENTIAL_VALUE, Fixture, HELPER, assert_no_credential_fragment,
    create_private_dir, expected_flood_prefix, fixture, run_test_scenario_in_child, sorted_entries,
};

/// Env var selecting the isolated file-failure scenario in the re-exec'd
/// child, carrying the completion marker path the parent checks. The exact
/// child test name keeps the process-wide `RLIMIT_FSIZE`/`SIGXFSZ` changes
/// confined to the child process.
const FILE_FAIL_CHILD_ENV: &str = "SAYA_C02_STORE_FILE_FAIL_DONE";
const FILE_FAIL_CHILD_TEST: &str =
    "c02::store::flood_with_unwritable_store_drains_child_and_reports_store_error";

/// The red test: flooding both streams bounds each stored file at the cap
/// plus the one truncation marker, keeps the head byte-identical to what the
/// child wrote first, and still reports the whole run honestly.
#[tokio::test]
async fn output_flood_is_bounded_and_resumable() {
    let Some(fixture) = fixture() else { return };
    let owner = CommandOwner::new(1).unwrap();
    let session = owner
        .start_with_output(
            fixture.run_program(),
            HELPER.to_owned(),
            vec!["flood".into(), "3".into(), "2".into()],
            NonZeroU64::new(30).unwrap(),
            fixture.marker("store-flood"),
        )
        .unwrap();
    let store_dir = session
        .output_dir()
        .expect("session carries an output store");
    let outcome = tokio::time::timeout(Duration::from_secs(30), session.join())
        .await
        .expect("the flood run settles within 30 s")
        .unwrap();
    assert_eq!(outcome.exit_code, Some(0), "the flood helper exits cleanly");
    let stdout_total: usize = 3 * 1024 * 1024;
    let stderr_total: usize = 2 * 1024 * 1024;
    assert_eq!(outcome.stdout.total_bytes, stdout_total as u64);
    assert_eq!(
        outcome.stdout.dropped_bytes,
        (stdout_total - OUTPUT_STORE_CAP_BYTES) as u64
    );
    assert_eq!(outcome.stderr.total_bytes, stderr_total as u64);
    assert_eq!(
        outcome.stderr.dropped_bytes,
        (stderr_total - OUTPUT_STORE_CAP_BYTES) as u64
    );
    for (stream, total, upper) in [
        (OutputStream::Stdout, stdout_total, false),
        (OutputStream::Stderr, stderr_total, true),
    ] {
        let bytes = fs::read(store_dir.join(stream.file_name()))
            .unwrap_or_else(|error| panic!("the {:?} store file reads: {error}", stream));
        assert!(
            bytes.ends_with(TRUNCATION_MARKER.as_bytes()),
            "a flooded stream ends with the one truncation marker"
        );
        let stored = &bytes[..bytes.len() - TRUNCATION_MARKER.len()];
        assert!(
            stored.len() <= OUTPUT_STORE_CAP_BYTES,
            "the stored head {} exceeds the cap",
            stored.len()
        );
        // Every pump chunk is at most MAX_OUTPUT_READ_BYTES, so the first
        // non-fitting chunk leaves the head within one chunk of the cap.
        assert!(
            stored.len() + MAX_OUTPUT_READ_BYTES > OUTPUT_STORE_CAP_BYTES,
            "the stored head is within one pump chunk of the cap: {}",
            stored.len()
        );
        assert!(
            stored.len() < total,
            "a megabyte flood cannot fit the cap: {} against {total}",
            stored.len()
        );
        assert_eq!(
            stored,
            expected_flood_prefix(stored.len(), upper).as_slice(),
            "the stored head is the stream's first bytes, byte-identical"
        );
    }
}

/// The declared credential value the helper echoes never reaches the store;
/// the env-name shape survives with its value scrubbed.
#[tokio::test]
async fn secret_is_redacted_before_it_reaches_the_store() {
    let Some(fixture) = fixture() else { return };
    let owner = CommandOwner::new(1).unwrap();
    let session = owner
        .start_with_output(
            fixture.run_program(),
            HELPER.to_owned(),
            vec!["echo-credential".into(), CREDENTIAL_ENV.into()],
            NonZeroU64::new(20).unwrap(),
            fixture.marker("store-secret"),
        )
        .unwrap();
    let store_dir = session
        .output_dir()
        .expect("session carries an output store");
    let outcome = tokio::time::timeout(Duration::from_secs(20), session.join())
        .await
        .expect("the echo run settles within 20 s")
        .unwrap();
    assert_eq!(outcome.exit_code, Some(0));
    let bytes = fs::read(store_dir.join(OutputStream::Stdout.file_name())).unwrap();
    let stored = String::from_utf8_lossy(&bytes).into_owned();
    assert!(
        !stored.contains(CREDENTIAL_VALUE),
        "the injected credential value must not reach the store: {stored:?}"
    );
    assert!(
        stored.contains(&format!("{CREDENTIAL_ENV}=[redacted]")),
        "the env-name shape survives with its value scrubbed: {stored:?}"
    );
}

/// A pre-placed symlink at the store's own path is refused, never adopted:
/// the start fails typed and nothing appears at the link's target.
#[tokio::test]
async fn pre_placed_symlink_at_store_path_is_refused() {
    let Some(fixture) = fixture() else { return };
    let store_root = fixture.marker("store-symlink");
    create_private_dir(&store_root);
    let target = store_root.join("symlink-target");
    create_private_dir(&target);
    std::os::unix::fs::symlink(&target, store_root.join("command-output")).unwrap();
    let owner = CommandOwner::new(1).unwrap();
    let result = owner.start_with_output(
        fixture.run_program(),
        HELPER.to_owned(),
        vec!["drip".into(), "1".into()],
        NonZeroU64::new(20).unwrap(),
        store_root,
    );
    assert!(
        matches!(result, Err(CommandStartError::OutputStore)),
        "a symlink at the store path is a typed refusal"
    );
    assert!(
        fs::read_dir(&target).unwrap().next().is_none(),
        "the symlink target gains no store entries"
    );
}

/// Without a store, `RunProgram::run` is unchanged: same outcome shape,
/// null stdin, no sink on the pump seam.
#[tokio::test]
async fn no_sink_run_program_is_unchanged() {
    let Some(fixture) = fixture() else { return };
    let line = "c02-drip 0\n";
    let outcome = tokio::time::timeout(
        Duration::from_secs(20),
        fixture
            .run_program()
            .run(HELPER, &["drip".into(), "1".into()], None),
    )
    .await
    .expect("the plain run settles within 20 s")
    .unwrap();
    assert_eq!(outcome.exit_code, Some(0));
    assert_eq!(outcome.stdout.text, line);
    assert!(!outcome.stdout.truncated);
    assert_eq!(outcome.stdout.dropped_bytes, 0);
    assert_eq!(outcome.stdout.total_bytes, line.len() as u64);
    assert!(!outcome.cancelled);
    assert!(!outcome.killed_by_timeout);
    assert!(outcome.signal.is_none());
}

/// A start refused after the store is prepared — a stopped owner's
/// admission — leaves no prepared store behind: the nonce directory is
/// removed, and the root gains no entries beyond the empty command-output
/// base it carried before the call.
#[tokio::test]
async fn refused_admission_leaves_no_store_directory() {
    let Some(fixture) = fixture() else { return };
    let store_root = fixture.marker("store-refused");
    // The base exists before the call, already private, so every entry the
    // refused start would leave behind is visible as a new name.
    create_private_dir(&store_root);
    create_private_dir(&store_root.join("command-output"));
    let before = sorted_entries(&store_root);
    let owner = CommandOwner::new(1).unwrap();
    owner.cancel_all();
    let result = owner.start_with_output(
        fixture.run_program(),
        HELPER.to_owned(),
        vec!["drip".into(), "1".into()],
        NonZeroU64::new(20).unwrap(),
        store_root.clone(),
    );
    assert!(
        matches!(result, Err(CommandStartError::OwnerStopped)),
        "a stopped owner refuses admission with the typed error"
    );
    assert_eq!(
        sorted_entries(&store_root),
        before,
        "the refused start adds no new directory entries at the store root"
    );
    assert!(
        fs::read_dir(store_root.join("command-output"))
            .unwrap()
            .next()
            .is_none(),
        "the prepared nonce directory is removed on refusal"
    );
}

/// End-to-end, what the test observes: the helper writes a credential in
/// two writes with a pause while the child is running — a wide benign
/// prefix, then the env name and the value's first half flushed — and the
/// store file never holds the value, nor any >= 8-byte fragment of it,
/// mid-run and after the run finishes. Which emission cut the pump
/// produced is not pinned here; the redactor's unit tests prove that.
#[tokio::test]
async fn credential_written_in_two_writes_with_a_pause_never_reaches_the_store() {
    let Some(fixture) = fixture() else { return };
    let sync_marker = fixture.marker("split-release");
    let first_half_marker = fixture.marker("split-first-half");
    let owner = CommandOwner::new(1).unwrap();
    let session = owner
        .start_with_output(
            fixture.run_program(),
            HELPER.to_owned(),
            vec![
                "split-credential".into(),
                CREDENTIAL_ENV.into(),
                sync_marker.to_string_lossy().into_owned(),
                first_half_marker.to_string_lossy().into_owned(),
            ],
            NonZeroU64::new(20).unwrap(),
            fixture.marker("store-split"),
        )
        .unwrap();
    let store_dir = session
        .output_dir()
        .expect("session carries an output store");
    let stdout_store = store_dir.join(OutputStream::Stdout.file_name());
    let store_len = || {
        fs::metadata(&stdout_store)
            .map(|meta| meta.len())
            .unwrap_or(0)
    };
    // The first-half marker exists only after the prefix, the env name
    // and the value's first half are flushed; the size must then hold
    // across two consecutive 50 ms polls before the mid-run read.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut previous = store_len();
    loop {
        while !first_half_marker.exists() {
            assert!(
                Instant::now() < deadline,
                "the helper's first-half marker appears within 5 s"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            Instant::now() < deadline,
            "the store file size settles within 5 s: {previous}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
        let current = store_len();
        if current == previous {
            break;
        }
        previous = current;
    }
    let mid_run = fs::read_to_string(&stdout_store).unwrap();
    assert_no_credential_fragment(&mid_run, "the mid-run store file");
    // Release the helper's second half, then assert the finished store.
    fs::write(&sync_marker, b"split").unwrap();
    let outcome = tokio::time::timeout(Duration::from_secs(20), session.join())
        .await
        .expect("the split run settles within 20 s")
        .unwrap();
    assert_eq!(outcome.exit_code, Some(0));
    let finished = fs::read_to_string(&stdout_store).unwrap();
    assert_no_credential_fragment(&finished, "the finished store file");
    assert!(
        finished.contains(&format!("{CREDENTIAL_ENV}=[redacted]")),
        "the env-name shape survives with its value scrubbed: {finished:?}"
    );
}

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
