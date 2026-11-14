//! The runner child's captured output: a ring buffer with byte caps, the
//! truncation report, and the one redaction gate every captured byte passes
//! before it can reach the model or disk.
//!
//! A child may emit gigabytes; the ring keeps the first and the last of a
//! stream inside one `cap`-byte budget — memory stays bounded by the cap no
//! matter what the child produces, and the middle that did not fit is
//! replaced by a marker naming how many bytes it was, so a long `npm
//! install` still shows both the error at its start and the exit at its end.
//! What was discarded is reported, never pretended away: the snapshot
//! carries the dropped byte count and the truncated flag, and a stream that
//! fits is rendered exactly as it was written, with no marker.
//!
//! Redaction happens here, at the only seam between the child's raw bytes
//! and anything a consumer — the model, the run's disk record — can read,
//! and it is unconditional: the credential-injection condition "redact()
//! applied to all captured output" is satisfied by construction, not by a
//! call site remembering to do it. The gate is two passes: the runner's own
//! value registry (the exact values it injected, scrubbed wholesale —
//! pattern redaction cannot recognise a bare value, measured by the exfiltration
//! battery) and then the pattern pass (`redact()`).

use std::{collections::VecDeque, io::Read, sync::Mutex};

#[path = "output_scrub.rs"]
mod scrub;
pub(crate) use scrub::secret_spans;

use saya_types::redact;

/// The byte cap per captured stream. Large enough for a real program's
/// diagnostics, small enough that a runaway child cannot fill the model's
/// context or the run's record with noise.
pub const OUTPUT_CAP_BYTES: usize = 64 * 1024;

/// The capture cap a shell uses when a result too large for the model's inline
/// bound saves its full output to the job's private state directory. One
/// mebibyte per stream: large enough for the diagnostics a real command
/// produces, and still an explicit ceiling. The model never reads this text
/// whole — the inline bound is much smaller — and the saved file is bounded
/// per file and per job by the writer.
pub const SHELL_SPOOL_CAPTURE_CAP_BYTES: usize = 1024 * 1024;

/// The read block a pump uses: output is drained continuously — the child
/// never blocks long on a full pipe — while the ring stays the only thing
/// that grows. Also the granularity the bounded read responses are sized
/// to.
pub(crate) const PUMP_CHUNK: usize = 8 * 1024;

/// The longest an elision marker can be: its literal text plus a twenty-digit
/// count. Sizes the one buffer a snapshot builds.
const MARKER_MAX_BYTES: usize = 64;

/// The marker that stands in the rendered text where the elided bytes were:
/// the exact count, so the model can tell how much of the stream it is not
/// seeing. Public because a reader that re-bounds rendered text of its own —
/// `web_fetch`, which bounds the *converted* text rather than the raw bytes —
/// must be able to name the same marker, both to recognise one a bound placed
/// and to place its own.
pub fn elision_marker(dropped: u64) -> String {
    format!("\n... [{dropped} bytes elided] ...\n")
}

/// The ring buffer for one stream: the first and last bytes it was fed
/// within `cap`, with the byte accounting the truncation report needs. The
/// head window fills first and never moves; the tail window keeps the newest
/// bytes, so a stream that outgrows the cap loses its middle and keeps both
/// ends. Shared across the reader threads; every mutation is behind the lock.
pub struct OutputRing {
    cap: usize,
    head_cap: usize,
    tail_cap: usize,
    state: Mutex<RingState>,
}

#[derive(Default)]
struct RingState {
    head: VecDeque<u8>,
    tail: VecDeque<u8>,
    total: u64,
    /// Newline-terminated lines this stream has written, cumulative across
    /// reads: a completion notice counts what the run produced, not what one
    /// read happened to take.
    lines: u64,
    /// Whether bytes after the last newline are waiting for one.
    partial: bool,
}

impl OutputRing {
    pub fn new(cap: usize) -> Self {
        let head_cap = cap.div_ceil(2);
        Self {
            cap,
            head_cap,
            tail_cap: cap - head_cap,
            state: Mutex::new(RingState::default()),
        }
    }

    /// Appends bytes: the head window takes them until it is full, then the
    /// tail window takes them, dropping its own oldest beyond its share. The
    /// total counts everything the child wrote, whether or not it was
    /// retained.
    pub fn push(&self, bytes: &[u8]) {
        let mut state = self.lock();
        state.total += bytes.len() as u64;
        state.lines += bytes.iter().filter(|&&byte| byte == b'\n').count() as u64;
        if let Some(&last) = bytes.last() {
            state.partial = last != b'\n';
        }
        let room = self.head_cap.saturating_sub(state.head.len());
        let (into_head, into_tail) = bytes.split_at(room.min(bytes.len()));
        state.head.extend(into_head.iter().copied());
        if !into_tail.is_empty() {
            state.tail.extend(into_tail.iter().copied());
            let overflow = state.tail.len().saturating_sub(self.tail_cap);
            if overflow > 0 {
                state.tail.drain(..overflow);
            }
        }
    }

    /// The retained bytes, lossily rendered, with the truncation report:
    /// `(text, truncated, dropped_bytes, total_bytes)`. A stream that fits
    /// renders exactly as it was written. One that does not renders its head,
    /// the marker naming the elided count, and its tail — and the tail gives
    /// up its own front, byte for byte, until the marker fits beside it, so
    /// the render stays inside the same cap. The marker is rendering, not
    /// stream bytes: `total = retained + dropped` holds either way, and the
    /// bytes the tail gave up are counted in `dropped` like any other.
    pub fn snapshot(&self) -> (String, bool, u64, u64) {
        render(&self.lock(), self.cap)
    }

    /// The same render, and clears the ring: the bytes it was fed since the
    /// last call. The read-and-clear a background run's reader uses — what it
    /// returns is exactly the output no earlier read has taken — over the same
    /// head-and-tail window and the same cap a foreground capture keeps.
    pub fn take(&self) -> (String, bool, u64, u64) {
        let mut state = self.lock();
        let (lines, partial) = (state.lines, state.partial);
        let taken = std::mem::replace(
            &mut *state,
            RingState {
                lines,
                partial,
                ..RingState::default()
            },
        );
        render(&taken, self.cap)
    }

    /// The lines this stream has written, cumulative across reads: one per
    /// newline, plus one for trailing bytes still waiting for theirs.
    pub fn lines(&self) -> u64 {
        let state = self.lock();
        state.lines + u64::from(state.partial)
    }

    /// Drains `reader` to EOF into this ring. Runs on a dedicated thread;
    /// EOF arrives when the child — and everything holding the write end —
    /// is gone.
    pub fn pump<R: Read>(&self, mut reader: R) {
        let mut chunk = vec![0u8; PUMP_CHUNK];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => self.push(&chunk[..n]),
            }
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, RingState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Renders one ring's state — its head, the elision marker when the cap threw
/// bytes away, and its tail — inside `cap`, with the honest report of what was
/// dropped. The one renderer both [`OutputRing::snapshot`] and
/// [`OutputRing::take`] use, so a read-and-clear reports exactly what a
/// snapshot of the same bytes would.
fn render(state: &RingState, cap: usize) -> (String, bool, u64, u64) {
    let mut keep_head = state.head.len();
    let mut keep_tail = state.tail.len();
    let mut dropped = state.total - (keep_head + keep_tail) as u64;
    // Each round trims at least one retained byte and lengthens the marker by
    // at most one digit, so this settles in a marker's worth of rounds; a cap
    // too small to hold the marker at all elides everything and reports the
    // whole stream as dropped.
    while dropped > 0 {
        let over = (keep_head + keep_tail + elision_marker(dropped).len()).saturating_sub(cap);
        let trim = over.min(keep_head + keep_tail);
        if trim == 0 {
            break;
        }
        let from_tail = trim.min(keep_tail);
        keep_tail -= from_tail;
        keep_head -= trim - from_tail;
        dropped += trim as u64;
    }
    let mut bytes = Vec::with_capacity(keep_head + MARKER_MAX_BYTES + keep_tail);
    bytes.extend(state.head.iter().copied().take(keep_head));
    if dropped > 0 {
        bytes.extend_from_slice(elision_marker(dropped).as_bytes());
    }
    bytes.extend(
        state
            .tail
            .iter()
            .copied()
            .skip(state.tail.len() - keep_tail),
    );
    (
        String::from_utf8_lossy(&bytes).into_owned(),
        dropped > 0,
        dropped,
        state.total,
    )
}

/// One finished stream: the redacted retained text — both ends of the stream
/// with the elision marker between them when it was truncated — plus the
/// honest report of what the cap threw away. The shape the outcome carries
/// to the model and the run's disk record.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct StreamCapture {
    pub text: String,
    pub truncated: bool,
    pub dropped_bytes: u64,
    pub total_bytes: u64,
}

impl StreamCapture {
    /// No bytes: the capture of a run that has not written through this
    /// stream yet.
    pub fn empty() -> Self {
        Self {
            text: String::new(),
            truncated: false,
            dropped_bytes: 0,
            total_bytes: 0,
        }
    }
}

/// Finalises one ring. This is the only function a captured stream's bytes
/// may leave through: the text is redacted here, before any consumer sees
/// it, unconditionally. Two passes, in this order:
///
/// 1. **The value registry.** `secrets` is the list of credential values
///    this child's environment was built from (`env::inject` returns it),
///    and every occurrence is replaced with the redaction marker before
///    anything else can fragment it. `redact()` is pattern-based — it needs
///    a marker (`password=`, a credential header, URL userinfo) to
///    recognise — so a value echoed bare (`SAYA_RUN_EP_ORCHESTRATOR=<value>`,
///    the exact shape an environment dump produces) would otherwise survive
///    to the model and the disk record. Measured by the exfiltration battery.
/// 2. **`redact()`.** The pattern pass every captured byte goes through:
///    credential-shaped markers, headers, userinfo URLs, private-key blocks.
///
/// A secret split across one of the ring's two window boundaries cannot be
/// matched as one string; the residual risk is stated here rather than
/// hidden. The marker between the windows keeps the two ends from being
/// joined into a value neither of them held.
pub fn capture(ring: &OutputRing, secrets: &[String]) -> StreamCapture {
    let (text, truncated, dropped_bytes, total_bytes) = ring.snapshot();
    StreamCapture {
        text: redact(&scrub_secrets(&text, secrets)),
        truncated,
        dropped_bytes,
        total_bytes,
    }
}

/// The same gate over a read-and-clear: the bytes written since the last call,
/// redacted exactly as [`capture`] redacts a finished stream. The one function
/// a background run's unread bytes leave through, so a polled stream passes the
/// same two passes — the value registry, then `redact()` — as a joined one.
pub fn take_capture(ring: &OutputRing, secrets: &[String]) -> StreamCapture {
    let (text, truncated, dropped_bytes, total_bytes) = ring.take();
    StreamCapture {
        text: redact(&scrub_secrets(&text, secrets)),
        truncated,
        dropped_bytes,
        total_bytes,
    }
}

/// Locates every occurrence of every non-empty secret in the ORIGINAL text,
/// including overlapping occurrences, and replaces the matches with the
/// redaction marker. Strictly overlapping spans merge into one marker;
/// adjacent non-overlapping spans keep one marker each. Empty values are
/// skipped: an empty pattern would match everywhere and destroy the capture
/// without protecting anything.
pub(crate) fn scrub_secrets(text: &str, secrets: &[String]) -> String {
    let spans = secret_spans(text, secrets);
    if spans.is_empty() {
        return text.to_owned();
    }
    let mut scrubbed = String::with_capacity(text.len());
    let mut cursor = 0;
    for (start, end) in spans {
        scrubbed.push_str(&text[cursor..start]);
        scrubbed.push_str("[redacted]");
        cursor = end;
    }
    scrubbed.push_str(&text[cursor..]);
    scrubbed
}

/// What one child run reported: the two captured streams and the honest
/// account of how the child ended. Exit code is data, not an error — a
/// non-zero exit is an `Ok` outcome carrying the code. Kills, cancellation,
/// truncation, and orphan sweeps are reported, never pretended away.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ProgramOutcome {
    pub program: String,
    pub pid: u32,
    pub exit_code: Option<i32>,
    /// The signal that killed the child, when one did.
    pub signal: Option<i32>,
    pub killed_by_timeout: bool,
    pub cancelled: bool,
    /// The child exited on its own but left live members in its process
    /// group (a daemonized grandchild); the group was killed and this says
    /// so. Nothing is left behind silently.
    pub killed_orphans: bool,
    pub duration_ms: u64,
    pub stdout: StreamCapture,
    pub stderr: StreamCapture,
}

#[cfg(test)]
#[path = "output_tests.rs"]
mod output_tests;

#[cfg(test)]
#[path = "output_scrub_tests.rs"]
mod output_scrub_tests;
