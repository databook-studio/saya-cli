//! Public-boundary tests of `CommandSession::read_output` (C02b): a cursor
//! resumes across concurrent appends, oversized and store-less reads are
//! typed refusals, a capped stream ends at the truncation marker, and a
//! refused redactor reads as an empty sealed stream.

use std::{
    num::NonZeroU64,
    time::{Duration, Instant},
};

use saya_harness::coding::commands::{
    CommandOutputCursor, CommandOutputError, CommandOutputStatus, CommandOwner, CommandSession,
    MAX_OUTPUT_READ_BYTES, OutputStream, TRUNCATION_MARKER,
};

use crate::output_capture::cursor_support;
use crate::support::{HELPER, expected_flood_prefix, fixture};

/// Polls `output_status` before any join, per the call-before-join
/// contract: `Some` only once both sinks finished, so every post-finish
/// read below observes the store's final outcome.
pub(crate) async fn wait_for_status(session: &CommandSession) -> CommandOutputStatus {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(ready) = session.output_status() {
            return ready;
        }
        assert!(Instant::now() < deadline, "status within 60 s");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Joins a finished session and asserts the helper exited cleanly.
pub(crate) async fn join_exits_cleanly(session: CommandSession) {
    let outcome = tokio::time::timeout(Duration::from_secs(30), session.join())
        .await
        .expect("the run settles within 30 s")
        .unwrap();
    assert_eq!(outcome.exit_code, Some(0), "the helper exits cleanly");
}

/// (a) Reads stdout of a still-appending helper through repeated bounded
/// reads: the cursor advances exactly by the bytes read (strictly when
/// bytes were read), the concatenation equals the helper's exact lines,
/// every response fits the read cap, and `end_of_stream` turns true only
/// once the sink has finished with the cursor at the stored length. The
/// helper sleeps 20 ms between drip lines, so with the 5 ms poll below a
/// non-empty read necessarily lands while `output_status` is still `None`;
/// `saw_live_nonempty` asserts that concurrent-append premise. The drip
/// volume must exceed the redactor's stage-B holdback
/// (`MIN_KEEP + KEEP_SLACK`, coding/commands/output_redact.rs): a stream
/// shorter than that window stores nothing until the writer closes, so
/// 200 lines (~2.5 KB) keep the emission cut open well before the run
/// ends.
#[tokio::test]
async fn cursor_reads_resume_across_concurrent_appends() {
    let Some(fixture) = fixture() else { return };
    let owner = CommandOwner::new(1).unwrap();
    let session = owner
        .start_with_output(
            fixture.run_program(),
            HELPER.to_owned(),
            vec!["drip".into(), "200".into()],
            NonZeroU64::new(30).unwrap(),
            fixture.marker("cursor-drip"),
        )
        .unwrap();
    let mut cursor = CommandOutputCursor::start(OutputStream::Stdout);
    let mut text = String::new();
    let mut saw_live_nonempty = false;
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        assert!(Instant::now() < deadline, "drip ends within 45 s");
        let read = session
            .read_output(cursor, MAX_OUTPUT_READ_BYTES)
            .expect("a bounded read of a live stream succeeds");
        assert!(
            read.text().len() <= MAX_OUTPUT_READ_BYTES,
            "every response fits the read cap: {}",
            read.text().len()
        );
        let next = read.next_cursor();
        assert_eq!(
            next.offset(),
            cursor.offset() + read.text().len() as u64,
            "the cursor advances exactly by the bytes read"
        );
        if session.output_status().is_none() && !read.text().is_empty() {
            saw_live_nonempty = true;
        }
        text.push_str(read.text());
        cursor = next;
        if read.end_of_stream() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let expected: String = (0..200).map(|i| format!("c02-drip {i}\n")).collect();
    assert_eq!(text, expected, "concatenation is the helper's exact lines");
    assert!(
        saw_live_nonempty,
        "a non-empty read happened while the sink was still running"
    );
    assert!(
        session.output_status().is_some(),
        "end-of-stream implies the sink finished before the last read"
    );
    join_exits_cleanly(session).await;
}

/// (b) Typed refusals: a read cap above `MAX_OUTPUT_READ_BYTES` is refused
/// as `ReadTooLarge` carrying the refused size, and a session started
/// without an output store refuses every read as `Unavailable`.
/// `CursorInvalid { offset }` is asserted in `cursor_edges` from a real
/// nonzero cursor resumed against a shortened stored stream file.
#[tokio::test]
async fn invalid_cursor_and_oversized_read_are_typed_refusals() {
    let Some(fixture) = fixture() else { return };
    let owner = CommandOwner::new(1).unwrap();
    let stored = owner
        .start_with_output(
            fixture.run_program(),
            HELPER.to_owned(),
            vec!["drip".into(), "1".into()],
            NonZeroU64::new(20).unwrap(),
            fixture.marker("cursor-refusals"),
        )
        .unwrap();
    wait_for_status(&stored).await;
    let refused = match stored.read_output(
        CommandOutputCursor::start(OutputStream::Stdout),
        MAX_OUTPUT_READ_BYTES + 1,
    ) {
        Ok(_) => panic!("a read cap above the maximum must be refused"),
        Err(CommandOutputError::ReadTooLarge { max_bytes }) => max_bytes,
        Err(other) => panic!("an oversized read is refused typed: {other:?}"),
    };
    assert_eq!(
        refused,
        MAX_OUTPUT_READ_BYTES + 1,
        "carries the refused cap"
    );
    join_exits_cleanly(stored).await;
    let plain = owner
        .start(
            fixture.run_program(),
            HELPER.to_owned(),
            vec!["drip".into(), "1".into()],
            NonZeroU64::new(20).unwrap(),
        )
        .unwrap();
    match plain.read_output(
        CommandOutputCursor::start(OutputStream::Stdout),
        MAX_OUTPUT_READ_BYTES,
    ) {
        Ok(_) => panic!("a session without a store must refuse reads"),
        Err(CommandOutputError::Unavailable) => {}
        Err(other) => panic!("a store-less read is refused as Unavailable: {other:?}"),
    }
    join_exits_cleanly(plain).await;
}

/// (c) Reads of a capped flood end exactly at the truncation marker: the
/// concatenation is the stored head byte-identically plus the one marker
/// line, every post-finish read reports the capped outcome with dropped
/// bytes, and end-of-stream lands only at the marker's end.
#[tokio::test]
async fn capped_stream_read_ends_at_marker_with_truncated() {
    let Some(fixture) = fixture() else { return };
    let owner = CommandOwner::new(1).unwrap();
    let session = owner
        .start_with_output(
            fixture.run_program(),
            HELPER.to_owned(),
            vec!["flood".into(), "2".into(), "0".into()],
            NonZeroU64::new(30).unwrap(),
            fixture.marker("cursor-flood"),
        )
        .unwrap();
    wait_for_status(&session).await;
    let mut cursor = CommandOutputCursor::start(OutputStream::Stdout);
    let mut text: Vec<u8> = Vec::new();
    let mut dropped_bytes;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        assert!(Instant::now() < deadline, "capped drains within 15 s");
        let read = session
            .read_output(cursor, MAX_OUTPUT_READ_BYTES)
            .expect("a bounded read of the finished store succeeds");
        assert!(
            read.truncated(),
            "a post-finish read of a capped stream reports truncation"
        );
        text.extend_from_slice(read.text().as_bytes());
        dropped_bytes = read.dropped_bytes();
        cursor = read.next_cursor();
        if read.end_of_stream() {
            break;
        }
    }
    let marker = TRUNCATION_MARKER.as_bytes();
    assert!(text.ends_with(marker), "ends at the truncation marker");
    let head = &text[..text.len() - marker.len()];
    assert_eq!(
        head,
        expected_flood_prefix(head.len(), false).as_slice(),
        "the reads see the stored head byte-identically"
    );
    assert!(
        dropped_bytes > 0,
        "bytes beyond the cap are reported dropped: {dropped_bytes}"
    );
    join_exits_cleanly(session).await;
}

/// (d) A declared credential of three bytes sits below the redactor's
/// minimum secret length, so the gate refuses at construction: the stream
/// is sealed from the first chunk — nothing stored, no marker — and the
/// start cursor reads as empty text at end-of-stream with the refused
/// outcome reported.
#[tokio::test]
async fn refused_redactor_stream_reads_empty_and_sealed() {
    let Some(fixture) = cursor_support::fixture() else {
        return;
    };
    let owner = CommandOwner::new(1).unwrap();
    let session = owner
        .start_with_output(
            fixture.run_program(),
            cursor_support::HELPER.to_owned(),
            vec![
                "echo-credential".into(),
                cursor_support::CREDENTIAL_ENV.into(),
            ],
            NonZeroU64::new(20).unwrap(),
            fixture.marker("cursor-refused"),
        )
        .unwrap();
    let status = wait_for_status(&session).await;
    assert!(
        status.capped && status.error.is_none(),
        "a refused redactor seals its stream without a store error: {status:?}"
    );
    assert!(matches!(
        session.capture_output(OutputStream::Stdout),
        Err(CommandOutputError::OutputNotComplete)
    ));
    let read = session
        .read_output(
            CommandOutputCursor::start(OutputStream::Stdout),
            MAX_OUTPUT_READ_BYTES,
        )
        .expect("reading a sealed stream at the start cursor succeeds");
    assert!(
        read.text().is_empty() && read.end_of_stream() && read.truncated(),
        "a sealed stream reads empty at end-of-stream with the refused outcome: {:?}",
        read.text()
    );
    join_exits_cleanly(session).await;
}
