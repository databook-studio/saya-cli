//! C02c lifecycle and secret-containment tests: a split credential never
//! reaches the output store mid-run or after finish, and a mid-flood stop
//! flushes the stored head, records the exit, and reaps in the recorded
//! order.

use std::{
    fs,
    num::NonZeroU64,
    path::Path,
    time::{Duration, Instant},
};

use saya_harness::coding::commands::{
    CommandOutputCursor, CommandOutputStatus, CommandOwner, CommandProcessState, CommandSession,
    MAX_OUTPUT_READ_BYTES, OutputStream, TRUNCATION_MARKER,
};

use crate::support::{CREDENTIAL_ENV, CREDENTIAL_VALUE, HELPER, fixture};

/// Polls `output_status` before any join, per the call-before-join contract:
/// `Some` only once both sinks finished.
async fn wait_for_status(session: &CommandSession) -> CommandOutputStatus {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(ready) = session.output_status() {
            return ready;
        }
        assert!(Instant::now() < deadline, "status within 60 s");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Mid-run drain: polls until at least one byte is readable, then reads until
/// a read yields nothing more, accumulating into `text`.
async fn drain_available(
    session: &CommandSession,
    mut cursor: CommandOutputCursor,
    text: &mut String,
) {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut seen = false;
    loop {
        assert!(Instant::now() < deadline, "readable bytes within 30 s");
        let read = session
            .read_output(cursor, MAX_OUTPUT_READ_BYTES)
            .expect("a bounded read of a live stream succeeds");
        let had_bytes = !read.text().is_empty();
        text.push_str(read.text());
        if seen && !had_bytes {
            return;
        }
        seen |= had_bytes;
        cursor = read.next_cursor();
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Finished drain: reads until end-of-stream, accumulating into `text`.
async fn drain_to_end(
    session: &CommandSession,
    mut cursor: CommandOutputCursor,
    text: &mut String,
) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        assert!(Instant::now() < deadline, "the stream drains within 30 s");
        let read = session
            .read_output(cursor, MAX_OUTPUT_READ_BYTES)
            .expect("a bounded read of the finished store succeeds");
        let finished = read.end_of_stream();
        text.push_str(read.text());
        if finished {
            return;
        }
        cursor = read.next_cursor();
    }
}

/// Bounded wait for the helper's first-half marker file: the helper writes
/// it only after flushing the benign prefix plus the first half of the
/// credential value, so a readable marker proves the first-half write has
/// landed before the mid-run snapshots look at the store — any readable
/// byte from the 5,000-byte benign prefix must not satisfy the wait.
async fn wait_for_first_half_marker(first_half: &Path) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(marker) = fs::read_to_string(first_half) {
            assert_eq!(marker, "first", "the helper's first-half marker text");
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the first-half marker within 30 s"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Asserts `text` carries neither the whole planted value nor ANY of its
/// eight-byte windows — every window at every offset, not just prefixes and
/// suffixes.
fn assert_no_credential_window(text: &str, context: &str) {
    assert!(
        !text.contains(CREDENTIAL_VALUE),
        "{context} holds the whole credential value: {text:?}"
    );
    for start in 0..=CREDENTIAL_VALUE.len() - 8 {
        let window = &CREDENTIAL_VALUE[start..start + 8];
        assert!(
            !text.contains(window),
            "{context} holds the 8-byte window {window:?} at offset {start}: {text:?}"
        );
    }
}

/// (a) The fixture's planted credential is split across a pump emission cut:
/// the test waits for the helper's first-half marker file before the mid-run
/// snapshots, in which neither the readable stdout nor the stored file
/// carries the value or any eight-byte window of it; after the second half
/// is released and the run finishes, neither stream read nor either stored
/// file carries the value or any such window, and the stored redaction text
/// appears instead.
#[tokio::test]
async fn secret_never_reaches_store_mid_run_or_after_finish() {
    let Some(fixture) = fixture() else { return };
    let owner = CommandOwner::new(1).unwrap();
    let release = fixture.marker("lifecycle-secret-release");
    let first_half = fixture.marker("lifecycle-secret-first");
    let session = owner
        .start_with_output(
            fixture.run_program(),
            HELPER.to_owned(),
            vec![
                "split-credential".into(),
                CREDENTIAL_ENV.into(),
                release.to_string_lossy().into_owned(),
                first_half.to_string_lossy().into_owned(),
            ],
            NonZeroU64::new(30).unwrap(),
            fixture.marker("lifecycle-secret"),
        )
        .unwrap();
    let dir = session.output_dir().expect("an output store exists");
    wait_for_first_half_marker(&first_half).await;
    let mut mid_text = String::new();
    drain_available(
        &session,
        CommandOutputCursor::start(OutputStream::Stdout),
        &mut mid_text,
    )
    .await;
    assert_no_credential_window(&mid_text, "the mid-run stdout read");
    let mid_stored = fs::read_to_string(dir.join("stdout")).expect("the stored stdout reads");
    assert_no_credential_window(&mid_stored, "the mid-run stored stdout");
    fs::write(&release, b"release").expect("the release marker writes");
    wait_for_status(&session).await;
    let mut stdout_text = String::new();
    drain_to_end(
        &session,
        CommandOutputCursor::start(OutputStream::Stdout),
        &mut stdout_text,
    )
    .await;
    let mut stderr_text = String::new();
    drain_to_end(
        &session,
        CommandOutputCursor::start(OutputStream::Stderr),
        &mut stderr_text,
    )
    .await;
    assert_no_credential_window(&stdout_text, "the finished stdout read");
    assert_no_credential_window(&stderr_text, "the finished stderr read");
    assert!(
        stdout_text.contains("[redacted]"),
        "the finished stdout read shows the redaction"
    );
    let outcome = tokio::time::timeout(Duration::from_secs(30), session.join())
        .await
        .expect("the run settles within 30 s")
        .unwrap();
    assert_eq!(outcome.exit_code, Some(0), "the helper exits cleanly");
    let final_stdout =
        fs::read_to_string(dir.join("stdout")).expect("the final stored stdout reads");
    let final_stderr =
        fs::read_to_string(dir.join("stderr")).expect("the final stored stderr reads");
    assert_no_credential_window(&final_stdout, "the final stored stdout");
    assert_no_credential_window(&final_stderr, "the final stored stderr");
    assert!(
        final_stdout.contains("[redacted]"),
        "the stored stdout shows the redaction"
    );
}

/// (b) A stop issued mid-flood, once the first bytes are readable: the test
/// reads `output_status` before any join, waits boundedly for `Reaped` and
/// asserts the status is already `Some` the moment `Reaped` is observed —
/// never the reverse, since the runner finishes the sinks before publishing
/// `Reaped` — the joined outcome records cancellation, and the stored head
/// is flushed — non-empty and ending at the truncation marker or a complete
/// line.
#[tokio::test]
async fn stop_mid_flood_flushes_before_reaped_and_records_exit() {
    let Some(fixture) = fixture() else { return };
    let owner = CommandOwner::new(1).unwrap();
    let session = owner
        .start_with_output(
            fixture.run_program(),
            HELPER.to_owned(),
            vec!["flood".into(), "1024".into(), "0".into()],
            NonZeroU64::new(30).unwrap(),
            fixture.marker("lifecycle-flood"),
        )
        .unwrap();
    let mut first_bytes = String::new();
    drain_available(
        &session,
        CommandOutputCursor::start(OutputStream::Stdout),
        &mut first_bytes,
    )
    .await;
    session.stop();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if matches!(session.process_state(), CommandProcessState::Reaped { .. }) {
            assert!(
                session.output_status().is_some(),
                "the moment Reaped is observed, output_status is already Some"
            );
            break;
        }
        assert!(Instant::now() < deadline, "Reaped within 60 s");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let dir = session.output_dir().expect("an output store exists");
    let outcome = tokio::time::timeout(Duration::from_secs(30), session.join())
        .await
        .expect("the stopped run settles within 30 s")
        .unwrap();
    assert!(
        outcome.cancelled,
        "the stopped run records a cancelled outcome"
    );
    let stored = fs::read_to_string(dir.join("stdout")).expect("the stored stdout reads");
    assert!(
        !stored.is_empty(),
        "stop flushed the readable bytes before reap"
    );
    assert!(
        stored.ends_with(TRUNCATION_MARKER) || stored.ends_with('\n'),
        "the stored head ends at the marker or a complete line, tail {:?}",
        &stored.as_bytes()[stored.len().saturating_sub(48)..]
    );
}
