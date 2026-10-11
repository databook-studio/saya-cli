//! The dedicated writer thread per stored stream: the thread owns the file
//! handle and the [`Redactor`], so the pump thread only hands raw chunks to a
//! bounded channel and never waits on storage or holds a lock across I/O.
//!
//! The thread appends redacted bytes until the stream cap, writes the one
//! truncation marker, then keeps draining the channel without writing — so a
//! capped stream still consumes the child's output and the child never blocks
//! on the store. Byte accounting is exact and saturating: every `write`
//! return value counts toward `stored_bytes` in redacted bytes; every whole
//! raw chunk drained after the cap, a write failure, or a redactor refusal
//! is counted in `discarded_raw_bytes` by its raw length; the invariant
//! with the sink is raw_total == (raw bytes handed to the redactor) +
//! dropped_bytes, where dropped counts channel-rejected and
//! writer-discarded whole chunks; all arithmetic saturates at `u64::MAX`
//! and never wraps.
//! The first I/O failure is recorded as `(kind, raw os code)` only — never
//! message text — after which the thread drains without writing and still
//! finishes with `sync_all` and the redacted tail.

use std::{
    fs, io,
    io::Write,
    sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use super::output::TRUNCATION_MARKER;
use super::output_redact::Redactor;
use super::output_snapshot::CapturedWrites;

/// Chunks the pump may hand a writer before a full channel seals the sink.
const CHANNEL_CAPACITY: usize = 32;

/// The poll interval while waiting out a writer's stop deadline.
const WRITER_STOP_POLL: Duration = Duration::from_millis(20);

/// What one writer thread did with its stream, typed without message text.
pub(super) struct WriterResult {
    /// Immutable host-memory bytes; never reconstructed from a mutable file.
    pub(super) captured_bytes: Option<std::sync::Arc<[u8]>>,
    /// Redacted bytes actually written, from `write` return values.
    pub(super) stored_bytes: u64,
    /// The stream was sealed before its end: the cap was hit (the file is
    /// a head prefix ending in the marker), or the redactor refused an
    /// over-long secret (the file stays empty, with no marker).
    pub(super) capped: bool,
    /// The first failure, `(kind, raw os code)`; a panicked writer is
    /// reported as `ErrorKind::Other` — never as a clean store.
    pub(super) error: Option<(io::ErrorKind, Option<i32>)>,
    /// Raw bytes of whole chunks drained and discarded after the cap was
    /// hit, a write failed, or the redactor refused: counted by raw length
    /// only; the redactor's held window is NOT counted here. The sink adds
    /// this to its channel-rejected drops, completing the invariant
    /// raw_total == (raw bytes handed to the redactor) + dropped_bytes,
    /// where dropped counts channel-rejected and writer-discarded whole
    /// chunks.
    pub(super) discarded_raw_bytes: u64,
}

/// One stream's writer: a bounded raw-chunk channel into its owner thread.
pub(super) struct StreamWriter {
    tx: SyncSender<Vec<u8>>,
    handle: JoinHandle<WriterResult>,
}

impl StreamWriter {
    /// Spawns the writer thread for one already-opened 0600 file. Spawn
    /// failure is a store failure the caller reports; nothing is written.
    pub(super) fn spawn(file: fs::File, secrets: &[String], cap: usize) -> io::Result<Self> {
        let (tx, rx) = sync_channel::<Vec<u8>>(CHANNEL_CAPACITY);
        let secrets = secrets.to_vec();
        let handle = thread::Builder::new()
            .name("saya-output-writer".into())
            .spawn(move || run_writer(file, rx, secrets, cap))?;
        Ok(Self { tx, handle })
    }

    /// Hands one raw chunk to the writer thread without ever blocking: a
    /// full channel is the caller's signal to seal and count the chunk
    /// dropped. This is a bounded-queue memory operation, never I/O.
    pub(super) fn try_send(&self, chunk: Vec<u8>) -> Result<(), TrySendError<Vec<u8>>> {
        self.tx.try_send(chunk)
    }

    /// Closes the channel — the writer drains it, appends its redacted
    /// tail, syncs, and exits — and returns the thread's typed result.
    pub(super) fn join(self) -> WriterResult {
        drop(self.tx);
        self.handle.join().unwrap_or(WriterResult {
            captured_bytes: None,
            stored_bytes: 0,
            capped: false,
            error: Some((io::ErrorKind::Other, None)),
            discarded_raw_bytes: 0,
        })
    }

    /// Closes the channel and waits for the writer to stop within
    /// `deadline`, polling `is_finished` every [`WRITER_STOP_POLL`]. A
    /// writer that stops in time is joined and its typed result returned;
    /// one that does not is detached — its handle drops here — and the
    /// caller records a `TimedOut` failure. The detached writer's further
    /// writes and counter updates are unaccounted by design: the sink has
    /// already taken its single slot and refuses to re-record. On timeout
    /// the counters are unknown and reported as 0.
    pub(super) fn join_bounded(self, deadline: Duration) -> WriterResult {
        drop(self.tx);
        let started = Instant::now();
        while !self.handle.is_finished() {
            if started.elapsed() >= deadline {
                return WriterResult {
                    captured_bytes: None,
                    stored_bytes: 0,
                    capped: false,
                    error: Some((io::ErrorKind::TimedOut, None)),
                    discarded_raw_bytes: 0,
                };
            }
            thread::sleep(WRITER_STOP_POLL);
        }
        self.handle.join().unwrap_or(WriterResult {
            captured_bytes: None,
            stored_bytes: 0,
            capped: false,
            error: Some((io::ErrorKind::Other, None)),
            discarded_raw_bytes: 0,
        })
    }
}

/// The writer loop: drain raw chunks, emit redacted bytes under the cap, and
/// finish with the redacted holdback and a `sync_all`. A redactor that
/// refuses at construction or mid-stream seals the stream as `capped` (no
/// marker) and everything after is counted dropped. Once the cap is hit or
/// a write fails, each later chunk is counted dropped by
/// its RAW length and is never handed to the redactor; the redactor's held
/// window is not counted because it was already handed over, completing the
/// invariant raw_total == (raw bytes handed to the redactor) + dropped_bytes,
/// where dropped counts channel-rejected and writer-discarded whole chunks,
/// so no drained raw byte goes unaccounted.
fn run_writer(
    mut file: fs::File,
    rx: Receiver<Vec<u8>>,
    secrets: Vec<String>,
    cap: usize,
) -> WriterResult {
    let mut redactor = Redactor::new(&secrets);
    let mut stored_bytes = CapturedWrites::new();
    let mut capped = redactor.is_refused();
    let mut error: Option<(io::ErrorKind, Option<i32>)> = None;
    let mut failed = false;
    let mut discarded_raw_bytes: u64 = 0;
    while let Ok(chunk) = rx.recv() {
        if capped || failed {
            discarded_raw_bytes = discarded_raw_bytes.saturating_add(chunk.len() as u64);
            continue;
        }
        let safe = redactor.push(&chunk);
        if redactor.is_refused() {
            // The redactor sealed itself mid-stream: store nothing more and
            // count every later chunk dropped, exactly like a cap seal.
            capped = true;
            continue;
        }
        emit(
            &mut file,
            &safe,
            cap,
            &mut stored_bytes,
            &mut capped,
            &mut error,
            &mut failed,
        );
    }
    let tail = redactor.finish();
    emit(
        &mut file,
        &tail,
        cap,
        &mut stored_bytes,
        &mut capped,
        &mut error,
        &mut failed,
    );
    if !failed {
        if let Err(source) = file.sync_all() {
            error = Some((source.kind(), source.raw_os_error()));
        }
    }
    WriterResult {
        stored_bytes: stored_bytes.stored_bytes(),
        captured_bytes: if capped || error.is_some() {
            None
        } else {
            stored_bytes.into_bytes()
        },
        capped,
        error,
        discarded_raw_bytes,
    }
}

/// Appends one redacted emission while the cap has room for it whole; on the
/// first overflow writes the one marker line and never writes again. A failed
/// stream stops writing entirely.
fn emit(
    file: &mut fs::File,
    bytes: &[u8],
    cap: usize,
    stored_bytes: &mut CapturedWrites,
    capped: &mut bool,
    error: &mut Option<(io::ErrorKind, Option<i32>)>,
    failed: &mut bool,
) {
    if *failed || *capped {
        return;
    }
    if stored_bytes
        .stored_bytes()
        .saturating_add(bytes.len() as u64)
        > cap as u64
    {
        *capped = true;
        write_counted(
            file,
            TRUNCATION_MARKER.as_bytes(),
            stored_bytes,
            error,
            failed,
        );
        return;
    }
    write_counted(file, bytes, stored_bytes, error, failed);
}

/// Writes `bytes` whole with exact partial-write accounting: `stored_bytes`
/// grows by exactly what `write` reported, the first error is recorded as
/// `(kind, raw os code)`, and the stream is marked failed.
fn write_counted(
    file: &mut fs::File,
    bytes: &[u8],
    stored_bytes: &mut CapturedWrites,
    error: &mut Option<(io::ErrorKind, Option<i32>)>,
    failed: &mut bool,
) {
    let mut written = 0;
    while written < bytes.len() {
        match file.write(&bytes[written..]) {
            Ok(0) => {
                *error = Some((io::ErrorKind::WriteZero, None));
                *failed = true;
                return;
            }
            Ok(n) => {
                stored_bytes.record(&bytes[written..written + n]);
                written += n;
            }
            Err(source) if source.kind() == io::ErrorKind::Interrupted => {}
            Err(source) => {
                *error = Some((source.kind(), source.raw_os_error()));
                *failed = true;
                return;
            }
        }
    }
}
