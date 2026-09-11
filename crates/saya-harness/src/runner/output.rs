//! The runner child's captured output: a ring buffer with byte caps, the
//! truncation report, and the one redaction gate every captured byte passes
//! before it can reach the model or disk.
//!
//! A child may emit gigabytes; the ring keeps the last `cap` bytes of a
//! stream and nothing more — memory stays bounded by the cap no matter what
//! the child produces. What was discarded is reported, never pretended away:
//! the snapshot carries the dropped byte count and the truncated flag.
//! Redaction happens here, at the only seam between the child's raw bytes
//! and anything a consumer — the model, the run's disk record — can read,
//! and it is unconditional: the credential-injection condition "redact()
//! applied to all captured output" is satisfied by construction, not by a
//! call site remembering to do it.

use std::{
    collections::VecDeque, io, io::Read, path::Path, sync::Mutex, time::SystemTime,
    time::UNIX_EPOCH,
};

use saya_types::redact;

use super::error::RunnerError;

/// The byte cap per captured stream. Large enough for a real program's
/// diagnostics, small enough that a runaway child cannot fill the model's
/// context or the run's record with noise.
pub const OUTPUT_CAP_BYTES: usize = 64 * 1024;

/// The read chunk a pump uses: output is drained continuously — the child
/// never blocks long on a full pipe — while the ring stays the only thing
/// that grows.
const PUMP_CHUNK: usize = 8 * 1024;

/// The ring buffer for one stream: the last `cap` bytes it was fed, with the
/// byte accounting the truncation report needs. Shared across the reader
/// threads; every mutation is behind the lock.
pub struct OutputRing {
    cap: usize,
    state: Mutex<RingState>,
}

#[derive(Default)]
struct RingState {
    tail: VecDeque<u8>,
    total: u64,
}

impl OutputRing {
    pub fn new(cap: usize) -> Self {
        Self {
            cap,
            state: Mutex::new(RingState::default()),
        }
    }

    /// Appends bytes, dropping the oldest beyond the cap. The total counts
    /// everything the child wrote, whether or not it was retained.
    pub fn push(&self, bytes: &[u8]) {
        let mut state = self.lock();
        state.total += bytes.len() as u64;
        state.tail.extend(bytes.iter().copied());
        let overflow = state.tail.len().saturating_sub(self.cap);
        if overflow > 0 {
            state.tail.drain(..overflow);
        }
    }

    /// The retained tail, lossily rendered, with the truncation report:
    /// `(text, truncated, dropped_bytes, total_bytes)`.
    pub fn snapshot(&self) -> (String, bool, u64, u64) {
        let state = self.lock();
        let bytes: Vec<u8> = state.tail.iter().copied().collect();
        (
            String::from_utf8_lossy(&bytes).into_owned(),
            state.total > bytes.len() as u64,
            state.total - bytes.len() as u64,
            state.total,
        )
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

/// One finished stream: the redacted retained text plus the honest report of
/// what the cap threw away. The shape the outcome carries to the model and
/// the run's disk record.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct StreamCapture {
    pub text: String,
    pub truncated: bool,
    pub dropped_bytes: u64,
    pub total_bytes: u64,
}

/// Finalises one ring. This is the only function a captured stream's bytes
/// may leave through: the text is redacted here, before any consumer sees
/// it, unconditionally.
pub fn capture(ring: &OutputRing) -> StreamCapture {
    let (text, truncated, dropped_bytes, total_bytes) = ring.snapshot();
    StreamCapture {
        text: redact(&text),
        truncated,
        dropped_bytes,
        total_bytes,
    }
}

/// Persists the redacted outcome into the run workspace — the disk record
/// the model's copy mirrors. Nothing unredacted exists to write: the record
/// serialises what `capture` already redacted.
pub fn record_outcome(dir: &Path, outcome: &ProgramOutcome) -> Result<String, RunnerError> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::fs::create_dir_all(dir).map_err(|source| RunnerError::RecordFailed { source })?;
    let path = dir.join(format!("{}-{}.json", nanos, outcome.program));
    let body = serde_json::to_vec_pretty(outcome).map_err(|error| RunnerError::RecordFailed {
        source: io::Error::new(io::ErrorKind::InvalidData, error.to_string()),
    })?;
    std::fs::write(&path, body).map_err(|source| RunnerError::RecordFailed { source })?;
    Ok(path.display().to_string())
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
mod tests {
    use super::*;

    #[test]
    fn a_ring_keeps_only_the_tail_and_reports_the_drop() {
        let ring = OutputRing::new(8);
        ring.push(b"0123456789");
        let (text, truncated, dropped, total) = ring.snapshot();
        assert_eq!(text, "23456789", "the two dropped head bytes are gone");
        assert!(truncated);
        assert_eq!(dropped, 2);
        assert_eq!(total, 10);
    }

    #[test]
    fn an_underfilled_ring_reports_no_truncation() {
        let ring = OutputRing::new(8);
        ring.push(b"ab");
        let (text, truncated, dropped, total) = ring.snapshot();
        assert_eq!(text, "ab");
        assert!(!truncated);
        assert_eq!(dropped, 0);
        assert_eq!(total, 2);
    }
}
