//! Edge reads at the public cursor boundary (C02b fix B): a real cursor
//! resumed past a shortened stored stream file is a typed `CursorInvalid`
//! carrying that cursor's offset, a removed stream file reads as typed
//! `Closed`, and a zero-byte read is empty and keeps the cursor exactly.

use std::{
    fs,
    num::NonZeroU64,
    path::{Path, PathBuf},
};

use saya_harness::coding::commands::{
    CommandOutputCursor, CommandOutputError, CommandOwner, MAX_OUTPUT_READ_BYTES, OutputStream,
};

use super::cursor::{join_exits_cleanly, wait_for_status};
use crate::support::{HELPER, fixture};

/// The stored stdout stream file: `output_dir` is the store's nonce-named
/// directory, and the per-stream files sit directly inside it named by
/// their stream.
fn stdout_stream_file(output_dir: &Path) -> PathBuf {
    output_dir.join("stdout")
}

/// (a) A real nonzero cursor taken from a finished read, resumed after the
/// stored stream file is truncated to zero bytes, is refused as
/// `CursorInvalid` carrying exactly that cursor's offset: the store bounds
/// the cursor against the stored length on every read.
#[tokio::test]
async fn cursor_past_stored_length_is_invalid() {
    let Some(fixture) = fixture() else { return };
    let owner = CommandOwner::new(1).unwrap();
    let stored = owner
        .start_with_output(
            fixture.run_program(),
            HELPER.to_owned(),
            vec!["drip".into(), "1".into()],
            NonZeroU64::new(20).unwrap(),
            fixture.marker("cursor-edge-invalid"),
        )
        .unwrap();
    wait_for_status(&stored).await;
    let first = stored
        .read_output(
            CommandOutputCursor::start(OutputStream::Stdout),
            MAX_OUTPUT_READ_BYTES,
        )
        .expect("a read of the finished one-line store succeeds");
    let cursor = first.next_cursor();
    assert!(
        cursor.offset() > 0,
        "the real cursor is nonzero: {cursor:?}"
    );
    let output_dir = stored
        .output_dir()
        .expect("the stored session has an output dir");
    let stream = stdout_stream_file(&output_dir);
    fs::OpenOptions::new()
        .write(true)
        .open(&stream)
        .expect("the stored stdout stream file opens for writing")
        .set_len(0)
        .expect("the stored stdout stream file truncates to zero bytes");
    let resumed = stored.read_output(cursor, MAX_OUTPUT_READ_BYTES);
    assert!(
        matches!(
            &resumed,
            Err(CommandOutputError::CursorInvalid { offset }) if *offset == cursor.offset()
        ),
        "a read past the shortened stored length is CursorInvalid at the same offset: {resumed:?}"
    );
    join_exits_cleanly(stored).await;
}

/// (b) A finished store whose stdout stream file is removed reads as typed
/// `Closed`: the no-follow open sees the missing file and maps it to the
/// closed-stream refusal, never a path or message text.
#[tokio::test]
async fn removed_stream_file_reads_as_closed() {
    let Some(fixture) = fixture() else { return };
    let owner = CommandOwner::new(1).unwrap();
    let stored = owner
        .start_with_output(
            fixture.run_program(),
            HELPER.to_owned(),
            vec!["drip".into(), "1".into()],
            NonZeroU64::new(20).unwrap(),
            fixture.marker("cursor-edge-removed"),
        )
        .unwrap();
    wait_for_status(&stored).await;
    let output_dir = stored
        .output_dir()
        .expect("the stored session has an output dir");
    fs::remove_file(stdout_stream_file(&output_dir))
        .expect("the stored stdout stream file removes");
    match stored.read_output(
        CommandOutputCursor::start(OutputStream::Stdout),
        MAX_OUTPUT_READ_BYTES,
    ) {
        Ok(_) => panic!("a read of a removed stream file must be refused"),
        Err(CommandOutputError::Closed) => {}
        Err(other) => panic!("a removed stream file reads as Closed: {other:?}"),
    }
    join_exits_cleanly(stored).await;
}

/// (c) A zero-byte read of a finished store is an empty read at the
/// cursor: empty text and a next cursor equal to the requested cursor —
/// same stream, same offset — so a zero cap advances nothing.
#[tokio::test]
async fn zero_byte_read_is_empty_and_keeps_the_cursor() {
    let Some(fixture) = fixture() else { return };
    let owner = CommandOwner::new(1).unwrap();
    let stored = owner
        .start_with_output(
            fixture.run_program(),
            HELPER.to_owned(),
            vec!["drip".into(), "1".into()],
            NonZeroU64::new(20).unwrap(),
            fixture.marker("cursor-edge-zero"),
        )
        .unwrap();
    wait_for_status(&stored).await;
    let cursor = CommandOutputCursor::start(OutputStream::Stdout);
    let read = stored
        .read_output(cursor, 0)
        .expect("a zero-byte read of the finished store succeeds");
    assert!(
        read.text().is_empty(),
        "a zero-byte read is empty: {:?}",
        read.text()
    );
    assert_eq!(
        read.next_cursor(),
        cursor,
        "the next cursor is the same stream and offset"
    );
    join_exits_cleanly(stored).await;
}
