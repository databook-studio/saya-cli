use std::time::{Duration, Instant};

use saya_types::RunEvent;

use super::{ElapsedClock, ElapsedClockError, ElapsedClockMark, system_time_ms};

impl ElapsedClock {
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
        let elapsed_at_start = Duration::from_millis(now_unix_ms - mark.origin_unix_ms);
        Ok(Self {
            mark: ElapsedClockMark {
                high_water_unix_ms: now_unix_ms,
                ..mark
            },
            ceiling,
            elapsed_at_start,
            elapsed: elapsed_at_start,
            started: now,
            last_journaled_high_water: mark.high_water_unix_ms,
            last_journaled_at: now,
            needs_journal: true,
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
}
