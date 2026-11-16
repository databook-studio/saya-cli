//! The call-level no-progress guard: consecutive settled calls whose result
//! moved nothing. It counts calls, not turns, because one productive call in a
//! turn hides the churn around it. It notes at the configured limit and again
//! at twice it, and stops at three times it, so a legitimate stretch of reads,
//! retries and inspections has room to run. The same shape as the repeat
//! detector, with a longer leash.

use super::{ProgressStop, ProgressStopKind};

/// What one idle call did to the guard.
#[derive(Debug)]
pub(super) enum CallOutcome {
    None,
    /// The streak reached a note point: one short note, no stop yet.
    Note {
        /// The idle-call count this note reports.
        count: u32,
        /// The count at which the job stops.
        stop_at: u32,
        /// This is the second and last note before the stop.
        last: bool,
    },
    /// The streak reached three times the limit.
    Stop(ProgressStop),
}

/// The consecutive-idle-call streak. The configured no-progress bound is the
/// same one the turn guard uses, so an operator has one key to raise.
#[derive(Debug, Default)]
pub(super) struct CallGuard {
    streak: u32,
    /// How many notes the current streak has emitted (at most two).
    notes: u32,
}

impl CallGuard {
    /// A call that changed something clears the streak.
    pub(super) fn progress(&mut self) {
        self.streak = 0;
        self.notes = 0;
    }

    /// A call that changed nothing extends the streak. `limit` is the
    /// configured no-progress bound; `None` or zero disables the guard. The
    /// guard notes at `limit` and `2 * limit`, and stops at `3 * limit`.
    pub(super) fn idle(&mut self, limit: Option<u32>, prefix: &'static str) -> CallOutcome {
        let Some(limit) = limit.filter(|limit| *limit > 0) else {
            return CallOutcome::None;
        };
        self.streak = self.streak.saturating_add(1);
        let stop_at = limit.saturating_mul(3);
        if self.streak >= stop_at {
            return CallOutcome::Stop(ProgressStop {
                kind: ProgressStopKind::NoProgress,
                limit: format!("{prefix}.no_progress_turns"),
                limit_value: Some(u64::from(limit)),
                measured: u64::from(self.streak),
                message: format!(
                    "stopped: no material progress for {} calls; change approach or raise {prefix}.no_progress_turns",
                    self.streak
                ),
            });
        }
        let note_at = match self.notes {
            0 => limit,
            1 => limit.saturating_mul(2),
            _ => return CallOutcome::None,
        };
        if self.streak < note_at {
            return CallOutcome::None;
        }
        self.notes = self.notes.saturating_add(1);
        CallOutcome::Note {
            count: note_at,
            stop_at,
            last: self.notes > 1,
        }
    }
}
