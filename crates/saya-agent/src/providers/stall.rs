//! The content-progress clock shared by the streaming providers.
//!
//! The transport idle timeout bounds the gap between *chunks*. A gateway can
//! keep that gap small with keep-alives, empty deltas, or usage-only frames
//! while the model produces nothing, so a second clock measures the gap
//! between content-bearing deltas: text, reasoning, or tool-call arguments.

use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct ProgressClock {
    stall: Duration,
    last: Option<Instant>,
}

impl ProgressClock {
    pub(super) fn new(stall: Duration) -> Self {
        Self { stall, last: None }
    }

    /// A content, reasoning, or tool-call delta arrived.
    pub(super) fn observe(&mut self) {
        self.last = Some(Instant::now());
    }

    /// When the stall window expires. Before the first delta the window runs
    /// from the stream's start, so a request that never produces anything is
    /// bounded too.
    pub(super) fn deadline(&self, started: Instant) -> Instant {
        self.last.unwrap_or(started) + self.stall
    }
}
