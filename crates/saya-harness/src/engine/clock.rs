//! Durable whole-run elapsed time reconstructed from the run journal.

use std::time::{Duration, Instant};

use saya_types::RunEvent;

mod replay;

const JOURNAL_CADENCE: Duration = Duration::from_secs(1);

/// An invalid or unavailable first-party clock cannot grant more budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ElapsedClockError {
    #[error("the run journal has no elapsed clock origin")]
    MissingOrigin,
    #[error("the run journal has inconsistent elapsed clock observations")]
    InvalidJournal,
    #[error("the system clock is earlier than the recorded clock origin")]
    FutureOrigin,
    #[error("the system clock moved behind its recorded high-water mark")]
    Backwards,
    #[error("the system clock is unavailable")]
    Unavailable,
}

/// A run's immutable UTC origin plus its durable maximum observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElapsedClockMark {
    origin_unix_ms: u64,
    high_water_unix_ms: u64,
}

/// Whole-run elapsed clock, combining durable UTC observations with monotonic
/// elapsed time during this invocation.
pub struct ElapsedClock {
    mark: ElapsedClockMark,
    ceiling: Duration,
    elapsed_at_start: Duration,
    elapsed: Duration,
    started: Instant,
    last_journaled_high_water: u64,
    last_journaled_at: Instant,
    needs_journal: bool,
}

impl ElapsedClock {
    /// Arm a fresh approved run at the current UTC millisecond.
    pub fn arm(
        ceiling: Duration,
        now_unix_ms: u64,
        now: Instant,
    ) -> Result<Self, ElapsedClockError> {
        if now_unix_ms == 0 {
            return Err(ElapsedClockError::Unavailable);
        }
        Ok(Self {
            mark: ElapsedClockMark {
                origin_unix_ms: now_unix_ms,
                high_water_unix_ms: now_unix_ms,
            },
            ceiling,
            elapsed_at_start: Duration::ZERO,
            elapsed: Duration::ZERO,
            started: now,
            last_journaled_high_water: now_unix_ms,
            last_journaled_at: now,
            needs_journal: true,
        })
    }

    /// Arm using the production UTC clock.
    pub fn arm_now(ceiling: Duration, now: Instant) -> Result<Self, ElapsedClockError> {
        Self::arm(ceiling, system_time_ms()?, now)
    }

    pub fn remaining(&self) -> Duration {
        self.ceiling.saturating_sub(self.elapsed)
    }

    pub fn event(&self) -> RunEvent {
        RunEvent::WallClockObserved {
            origin_unix_ms: self.mark.origin_unix_ms,
            high_water_unix_ms: self.mark.high_water_unix_ms,
        }
    }

    /// Raise the mark using both UTC and invocation-local monotonic time.
    pub fn observe(&mut self, now_unix_ms: u64, now: Instant) -> Result<bool, ElapsedClockError> {
        let monotonic_elapsed = self
            .elapsed_at_start
            .saturating_add(now.saturating_duration_since(self.started));
        self.elapsed = self.elapsed.max(monotonic_elapsed);
        self.elapsed = self.elapsed.max(Duration::from_millis(
            now_unix_ms.saturating_sub(self.mark.origin_unix_ms),
        ));
        if now_unix_ms < self.mark.high_water_unix_ms {
            return Err(ElapsedClockError::Backwards);
        }
        let monotonic_ms = u64::try_from(monotonic_elapsed.as_millis())
            .map_err(|_| ElapsedClockError::Unavailable)?;
        let monotonic_high = self
            .mark
            .origin_unix_ms
            .checked_add(monotonic_ms)
            .ok_or(ElapsedClockError::Unavailable)?;
        let previous_high_water = self.mark.high_water_unix_ms;
        let high_water = now_unix_ms.max(monotonic_high);
        self.mark.high_water_unix_ms = high_water;
        self.elapsed = self.elapsed.max(Duration::from_millis(
            high_water.saturating_sub(self.mark.origin_unix_ms),
        ));
        Ok(high_water > previous_high_water)
    }

    pub(crate) fn observation_due(&self, now: Instant, force: bool) -> bool {
        if self.needs_journal {
            return true;
        }
        if self.mark.high_water_unix_ms <= self.last_journaled_high_water {
            return false;
        }
        force || now.saturating_duration_since(self.last_journaled_at) >= JOURNAL_CADENCE
    }

    pub(crate) fn mark_persisted(&mut self, now: Instant) {
        self.last_journaled_high_water = self.mark.high_water_unix_ms;
        self.last_journaled_at = now;
        self.needs_journal = false;
    }
}

pub(super) fn system_time_ms() -> Result<u64, ElapsedClockError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .filter(|millis| *millis != 0)
        .ok_or(ElapsedClockError::Unavailable)
}
