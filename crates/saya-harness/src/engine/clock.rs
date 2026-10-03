//! Durable whole-run elapsed time reconstructed from the run journal.

use std::time::{Duration, Instant};

use saya_types::RunEvent;

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
        })
    }

    /// Arm using the production UTC clock.
    pub fn arm_now(ceiling: Duration, now: Instant) -> Result<Self, ElapsedClockError> {
        Self::arm(ceiling, system_time_ms()?, now)
    }

    /// Resume only when the repaired journal has one consistent clock history.
    pub fn resume(
        events: &[RunEvent],
        ceiling: Duration,
        now_unix_ms: u64,
        now: Instant,
    ) -> Result<Self, ElapsedClockError> {
        let mut mark = None;
        let mut started = false;
        let mut approved = false;
        for event in events {
            match event {
                RunEvent::RunStarted => started = true,
                RunEvent::PlanApproved { .. } if started => approved = true,
                RunEvent::WallClockObserved { .. } if !started || !approved => {
                    return Err(ElapsedClockError::InvalidJournal);
                }
                _ => {}
            }
            let RunEvent::WallClockObserved {
                origin_unix_ms,
                high_water_unix_ms,
            } = event
            else {
                continue;
            };
            if *origin_unix_ms == 0
                || *high_water_unix_ms < *origin_unix_ms
                || mark.is_some_and(|previous: ElapsedClockMark| {
                    previous.origin_unix_ms != *origin_unix_ms
                        || *high_water_unix_ms < previous.high_water_unix_ms
                })
            {
                return Err(ElapsedClockError::InvalidJournal);
            }
            mark = Some(ElapsedClockMark {
                origin_unix_ms: *origin_unix_ms,
                high_water_unix_ms: *high_water_unix_ms,
            });
        }
        let mark = mark.ok_or(ElapsedClockError::MissingOrigin)?;
        if now_unix_ms < mark.origin_unix_ms {
            return Err(ElapsedClockError::FutureOrigin);
        }
        if now_unix_ms < mark.high_water_unix_ms {
            return Err(ElapsedClockError::Backwards);
        }
        let elapsed_ms = now_unix_ms.saturating_sub(mark.origin_unix_ms);
        let elapsed_at_start = Duration::from_millis(elapsed_ms);
        Ok(Self {
            mark: ElapsedClockMark {
                high_water_unix_ms: now_unix_ms,
                ..mark
            },
            ceiling,
            elapsed_at_start,
            elapsed: elapsed_at_start,
            started: now,
        })
    }

    /// Resume using the production UTC clock.
    pub fn resume_now(
        events: &[RunEvent],
        ceiling: Duration,
        now: Instant,
    ) -> Result<Self, ElapsedClockError> {
        Self::resume(events, ceiling, system_time_ms()?, now)
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
        if now_unix_ms < self.mark.high_water_unix_ms {
            return Err(ElapsedClockError::Backwards);
        }
        let elapsed = self
            .elapsed_at_start
            .saturating_add(now.saturating_duration_since(self.started));
        let monotonic_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
        let monotonic_high = self.mark.origin_unix_ms.saturating_add(monotonic_ms);
        let high_water = now_unix_ms.max(monotonic_high);
        let changed = high_water > self.mark.high_water_unix_ms;
        self.mark.high_water_unix_ms = high_water;
        self.elapsed = self.elapsed.max(Duration::from_millis(
            high_water.saturating_sub(self.mark.origin_unix_ms),
        ));
        Ok(changed)
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
