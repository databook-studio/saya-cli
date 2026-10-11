//! The per-stream append sink the pump seam feeds for one command run's
//! output store. The sink owns no file and does no I/O on the pump path: it
//! counts the child's raw bytes, hands each raw chunk to its stream's writer
//! thread through a bounded channel, and — when the channel is full, the
//! writer failed to spawn, or the store already finished — seals itself and
//! counts every later chunk dropped. A sealed store's file is a clean head
//! prefix with no mid-stream gap: bytes are delivered whole or not at all.
//!
//! The writer thread (see [`super::output_writer`]) owns the file handle and
//! the redactor, so the pump thread never waits on storage and no lock here
//! is ever held across I/O. `finish` runs after the pump joined, joins the
//! store's own writer thread, and records the typed result `output_status`
//! reports; the old final full-file re-scrub is gone — published bytes and
//! the truncation marker never move. `finish_bounded` is the failed-wait
//! variant: it bounds the writer join and, on timeout, detaches the writer
//! and records a `TimedOut` failure instead of blocking the caller.

use std::{
    fs, io,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use crate::proc::RawChunkSink;

use super::output::OUTPUT_STORE_CAP_BYTES;
use super::output_writer::StreamWriter;

/// One stream's final store accounting, typed without message text.
///
/// Invariant over the whole stream: `raw_total_bytes` counts every raw byte
/// the pump observed, and `dropped_bytes` counts raw bytes of WHOLE chunks
/// never handed to the redactor — channel-rejected after a seal, or
/// discarded by the writer after the cap/failure/refusal — therefore
/// `raw_total == handed + dropped` exactly in raw bytes, where `handed` is
/// the writer's internal count of raw bytes handed to the redactor (see
/// [`super::output_writer`]; when the writer was joined; after a
/// bounded-finish timeout `handed` is unknown and reported 0).
/// `stored_bytes` is redacted bytes written, a different unit, never mixed
/// with raw counts; `capped` says handed bytes were not all stored;
/// counters saturate at `u64::MAX`.
#[derive(Clone)]
pub(super) struct SinkResult {
    pub(super) captured_bytes: Option<std::sync::Arc<[u8]>>,
    pub(super) stored_bytes: u64,
    /// Channel-rejected raw bytes plus writer-discarded raw bytes.
    pub(super) dropped_bytes: u64,
    pub(super) raw_total_bytes: u64,
    pub(super) capped: bool,
    pub(super) error: Option<(io::ErrorKind, Option<i32>)>,
}

/// The writer slot for one stream: the eagerly opened 0600 file until the
/// first chunk spawns the writer, then the writer itself, then the typed
/// finish result.
enum WriterSlot {
    #[cfg_attr(not(unix), allow(dead_code))]
    Unstarted(fs::File),
    Running(StreamWriter),
    Failed((io::ErrorKind, Option<i32>)),
    Finished(SinkResult),
}

/// One stream's sealed delivery path into its writer thread. `sealed` is
/// permanent: once delivery cannot be trusted to stay gapless, every later
/// chunk is counted dropped.
pub(super) struct StoreSink {
    state: Mutex<Option<WriterSlot>>,
    raw_total_bytes: AtomicU64,
    dropped_bytes: AtomicU64,
    sealed: AtomicBool,
}

impl RawChunkSink for StoreSink {
    /// Counts the raw chunk and hands it to the writer thread. Never blocks
    /// and never touches the file: a full channel seals the sink, and the
    /// state lock is held only across in-memory queue operations.
    fn observe(&self, chunk: &[u8], secrets: &[String]) {
        let len = chunk.len() as u64;
        count(&self.raw_total_bytes, len);
        if self.sealed.load(Ordering::Acquire) {
            count(&self.dropped_bytes, len);
            return;
        }
        let mut slot = self.lock();
        if let Some(WriterSlot::Unstarted(_)) = slot.as_ref() {
            let spawned = match slot.take() {
                Some(WriterSlot::Unstarted(file)) => {
                    StreamWriter::spawn(file, secrets, OUTPUT_STORE_CAP_BYTES)
                }
                _ => unreachable!("matched Unstarted above"),
            };
            match spawned {
                Ok(writer) => *slot = Some(WriterSlot::Running(writer)),
                Err(source) => {
                    *slot = Some(WriterSlot::Failed((source.kind(), source.raw_os_error())));
                }
            }
        }
        let delivered = match slot.as_mut() {
            Some(WriterSlot::Running(writer)) => writer.try_send(chunk.to_vec()).is_ok(),
            _ => false,
        };
        drop(slot);
        if !delivered {
            self.seal(len);
        }
    }

    /// Finalises the store after the pump joined: closes the channel, joins
    /// the stream's own writer thread, and records the typed result. The
    /// writer can only drain buffered chunks and write within the cap, so
    /// this never waits on the child; a still-running pump from a bounded
    /// finish finds the slot taken and is counted dropped.
    fn finish(&self, _secrets: &[String]) {
        self.finish_impl(None);
    }

    /// Bounded failed-wait finalisation: closes the channel and gives the
    /// writer `deadline` to stop, polling `is_finished`. A writer that
    /// stops in time is joined and recorded normally; one that does not is
    /// detached and the store is recorded failed with `ErrorKind::TimedOut`
    /// and no message text. The detached writer cannot alter the recorded
    /// result afterwards: the single slot is already taken, re-recording is
    /// refused, and late observes are only counted (raw and dropped)
    /// against the already-frozen result.
    fn finish_bounded(&self, deadline: Duration) {
        self.finish_impl(Some(deadline));
    }
}

impl StoreSink {
    /// Wraps one already-opened, exclusively created 0600 file. The file
    /// stays here until the first chunk moves it into the writer thread.
    #[cfg(unix)]
    pub(super) fn new(file: fs::File) -> Self {
        Self {
            state: Mutex::new(Some(WriterSlot::Unstarted(file))),
            raw_total_bytes: AtomicU64::new(0),
            dropped_bytes: AtomicU64::new(0),
            sealed: AtomicBool::new(false),
        }
    }

    /// The typed result `output_status` reports once `finish` ran.
    pub(super) fn result(&self) -> Option<SinkResult> {
        let slot = self.lock();
        match slot.as_ref() {
            Some(WriterSlot::Finished(result)) => Some(result.clone()),
            _ => None,
        }
    }

    /// The raw total the cursor reader reports while the stream still
    /// writes, straight from the atomic.
    #[cfg(unix)]
    pub(super) fn running_raw_total_bytes(&self) -> u64 {
        self.raw_total_bytes.load(Ordering::Acquire)
    }

    /// The channel-rejected drops the cursor reader reports while the
    /// stream still writes, straight from the atomic.
    #[cfg(unix)]
    pub(super) fn running_dropped_bytes(&self) -> u64 {
        self.dropped_bytes.load(Ordering::Acquire)
    }

    /// Shared finalisation: takes the single writer slot once, joins the
    /// writer unboundedly or within the bound, and records the typed
    /// result. Drop accounting is summed here: `dropped_bytes` is the
    /// channel-rejected raw bytes counted by `observe` plus the raw bytes
    /// the writer itself discarded after the cap or a failure. The slot is
    /// taken exactly once, so a late finish or a detached writer cannot
    /// re-record or alter the stored `SinkResult`; late observes keep
    /// counting into the atomics but never change the recorded result.
    fn finish_impl(&self, bounded: Option<Duration>) {
        let mut slot = self.lock();
        if matches!(slot.as_ref(), Some(WriterSlot::Finished(_))) {
            return;
        }
        let taken = slot.take();
        drop(slot);
        let result = match taken {
            Some(WriterSlot::Running(writer)) => {
                let written = match bounded {
                    Some(deadline) => writer.join_bounded(deadline),
                    None => writer.join(),
                };
                SinkResult {
                    captured_bytes: written.captured_bytes,
                    stored_bytes: written.stored_bytes,
                    dropped_bytes: self
                        .dropped_bytes
                        .load(Ordering::Acquire)
                        .saturating_add(written.discarded_raw_bytes),
                    raw_total_bytes: self.raw_total_bytes.load(Ordering::Acquire),
                    capped: written.capped,
                    error: written.error,
                }
            }
            Some(WriterSlot::Unstarted(file)) => {
                // Nothing was written, so there is nothing to sync.
                drop(file);
                let error = None;
                SinkResult {
                    captured_bytes: Some(std::sync::Arc::from([])),
                    stored_bytes: 0,
                    dropped_bytes: self.dropped_bytes.load(Ordering::Acquire),
                    raw_total_bytes: self.raw_total_bytes.load(Ordering::Acquire),
                    capped: false,
                    error,
                }
            }
            Some(WriterSlot::Failed(source)) => SinkResult {
                captured_bytes: None,
                stored_bytes: 0,
                dropped_bytes: self.dropped_bytes.load(Ordering::Acquire),
                raw_total_bytes: self.raw_total_bytes.load(Ordering::Acquire),
                capped: false,
                error: Some(source),
            },
            Some(WriterSlot::Finished(result)) => result,
            None => return,
        };
        let mut slot = self.lock();
        if slot.is_none() {
            *slot = Some(WriterSlot::Finished(result));
        }
    }

    /// Seals the sink permanently and counts this chunk dropped.
    fn seal(&self, len: u64) {
        self.sealed.store(true, Ordering::Release);
        count(&self.dropped_bytes, len);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<WriterSlot>> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Checked atomic accumulation: saturates at `u64::MAX` instead of wrapping.
fn count(counter: &AtomicU64, len: u64) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        Some(value.saturating_add(len))
    });
}
