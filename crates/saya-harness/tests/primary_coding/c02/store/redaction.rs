use std::{
    fs,
    num::NonZeroU64,
    time::{Duration, Instant},
};

use saya_harness::coding::commands::{CommandOwner, OutputStream};

use crate::support::{
    CREDENTIAL_ENV, CREDENTIAL_VALUE, HELPER, assert_no_credential_fragment, fixture,
};

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
