//! The captured-stdio seam both lanes share: one `Pipes` pair of rings per
//! child, pumped on dedicated threads from spawn to EOF, plus the optional
//! raw-chunk sink pair a caller attaches to observe each chunk as the pump
//! reads it.
//!
//! The sink sees the child's raw bytes — before any redaction — which is
//! what makes it a seam: redaction-before-store is each sink's own policy
//! (the command output store's), not the pump's. With no sinks attached the
//! loop is exactly the pre-seam pump — read, push to the ring, nothing else.

use std::{
    io,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::runner::output::{OUTPUT_CAP_BYTES, OutputRing, PUMP_CHUNK, StreamCapture};

/// The poll interval while waiting out a pump's join deadline.
const PUMP_JOIN_POLL: Duration = Duration::from_millis(20);

/// The crate-private raw-chunk observer the pump feeds. A trait rather than
/// an enum of one variant because a second observer (the planned frame gate)
/// attaches through this same seam; there is exactly one production
/// implementation today.
pub(crate) trait RawChunkSink: Send + Sync {
    /// Observes one raw chunk the pump read, together with the credential
    /// values the child's environment was built from. What the observer does
    /// with them — redact, frame, count — is its own policy.
    fn observe(&self, chunk: &[u8], secrets: &[String]);

    /// Finalises the sink after its pump joined: the last chance to flush,
    /// re-scrub, or mark the stream ended. Runs once, before the outcome is
    /// published.
    fn finish(&self, secrets: &[String]);

    /// Bounded failed-wait finalisation: seals the sink within `deadline`
    /// instead of waiting unboundedly on its store. A store that does not
    /// stop in time is detached and recorded failed with a typed
    /// `TimedOut` error, never blocking the caller.
    fn finish_bounded(&self, deadline: Duration);
}

/// The optional sink pair threaded from the command session to the pumps.
pub(crate) struct OutputSinks {
    pub stdout: Option<Arc<dyn RawChunkSink>>,
    pub stderr: Option<Arc<dyn RawChunkSink>>,
}

/// The captured stdio rings for one child: stdout and stderr pumped on
/// dedicated threads from spawn until EOF, bounded by the shared cap.
pub struct Pipes {
    stdout: Arc<OutputRing>,
    stderr: Arc<OutputRing>,
    sinks: Option<OutputSinks>,
    pumps: Vec<std::thread::JoinHandle<()>>,
}

impl Pipes {
    /// Takes the child's pipes and starts the pumps. The pipes must be
    /// piped; a missing pipe is a wait failure, never a silent gap.
    pub fn take(
        stdout: &mut Option<impl io::Read + Send + 'static>,
        stderr: &mut Option<impl io::Read + Send + 'static>,
    ) -> io::Result<Self> {
        Self::take_with_sinks(stdout, stderr, None, &[])
    }

    /// `take` with an optional raw-chunk sink pair: each pump pushes to its
    /// ring and then hands the same raw chunk to its sink.
    pub(crate) fn take_with_sinks(
        stdout: &mut Option<impl io::Read + Send + 'static>,
        stderr: &mut Option<impl io::Read + Send + 'static>,
        sinks: Option<OutputSinks>,
        secrets: &[String],
    ) -> io::Result<Self> {
        let stdout_ring = Arc::new(OutputRing::new(OUTPUT_CAP_BYTES));
        let stderr_ring = Arc::new(OutputRing::new(OUTPUT_CAP_BYTES));
        let stdout_pipe = stdout
            .take()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "stdout not captured"))?;
        let stderr_pipe = stderr
            .take()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "stderr not captured"))?;
        let secrets: Arc<[String]> = Arc::from(secrets.to_vec());
        let (stdout_sink, stderr_sink) = match &sinks {
            Some(sinks) => (sinks.stdout.clone(), sinks.stderr.clone()),
            None => (None, None),
        };
        let pumps = vec![
            pump(
                Arc::clone(&stdout_ring),
                stdout_pipe,
                stdout_sink,
                Arc::clone(&secrets),
            ),
            pump(Arc::clone(&stderr_ring), stderr_pipe, stderr_sink, secrets),
        ];
        Ok(Self {
            stdout: stdout_ring,
            stderr: stderr_ring,
            sinks,
            pumps,
        })
    }

    /// Joins the pumps, finalises any sinks (their last chance to flush
    /// before the outcome is published), and redacts both rings through the
    /// single capture gate both lanes share.
    pub fn finish(self, secrets: &[String]) -> (StreamCapture, StreamCapture) {
        for pump in self.pumps {
            let _ = pump.join();
        }
        if let Some(sinks) = &self.sinks {
            if let Some(sink) = &sinks.stdout {
                sink.finish(secrets);
            }
            if let Some(sink) = &sinks.stderr {
                sink.finish(secrets);
            }
        }
        (
            crate::runner::output::capture(&self.stdout, secrets),
            crate::runner::output::capture(&self.stderr, secrets),
        )
    }

    /// Seals the pipes after a wait that failed: joins each pump within
    /// `deadline` so a child that still holds its pipe open cannot block
    /// this caller forever, then seals any attached sink through its
    /// bounded finish, and returns without capturing. `deadline` is the
    /// total budget shared across the pump joins and the store shutdown —
    /// each wait gets only what is left, so a stalled pump cannot eat the
    /// store's wait and a stalled store writer cannot block this caller.
    /// Pumps that do not finish in time are abandoned — their handles drop
    /// and the threads detach; a store writer that does not stop in time
    /// is detached by the sink and recorded as a `TimedOut` failure.
    pub fn finish_bounded(self, _secrets: &[String], deadline: Duration) {
        let started = Instant::now();
        for pump in self.pumps {
            join_bounded(pump, deadline.saturating_sub(started.elapsed()));
        }
        if let Some(sinks) = &self.sinks {
            if let Some(sink) = &sinks.stdout {
                sink.finish_bounded(deadline.saturating_sub(started.elapsed()));
            }
            if let Some(sink) = &sinks.stderr {
                sink.finish_bounded(deadline.saturating_sub(started.elapsed()));
            }
        }
    }
}

/// Joins one pump within `deadline`, polling `is_finished` so the wait
/// stays bounded; a pump that does not finish in time has its handle
/// dropped, which detaches the thread.
fn join_bounded(pump: std::thread::JoinHandle<()>, deadline: Duration) {
    let started = Instant::now();
    while !pump.is_finished() {
        if started.elapsed() >= deadline {
            return;
        }
        std::thread::sleep(PUMP_JOIN_POLL);
    }
    let _ = pump.join();
}

fn pump(
    ring: Arc<OutputRing>,
    pipe: impl io::Read + Send + 'static,
    sink: Option<Arc<dyn RawChunkSink>>,
    secrets: Arc<[String]>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || match sink {
        Some(sink) => pump_to_sink(ring, pipe, sink, secrets),
        None => ring.pump(pipe),
    })
}

/// The sink-bearing pump loop: read one chunk, push it to the ring (the
/// tail-keeping capture is unchanged), then hand the same raw chunk to the
/// sink. The ring is pushed first, so the capture never waits on a sink.
fn pump_to_sink(
    ring: Arc<OutputRing>,
    mut pipe: impl io::Read + Send + 'static,
    sink: Arc<dyn RawChunkSink>,
    secrets: Arc<[String]>,
) {
    let mut chunk = vec![0u8; PUMP_CHUNK];
    loop {
        match pipe.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                ring.push(&chunk[..n]);
                sink.observe(&chunk[..n], &secrets);
            }
        }
    }
}
