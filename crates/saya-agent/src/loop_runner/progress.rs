//! Progress-based stop guards for long-running jobs and run steps.
//!
//! Request and token counts say how much a run spent, not whether it is
//! getting anywhere: a healthy long job and a stuck one can spend the same.
//! This classifier watches the work itself — a successful workspace write, a
//! materially changed test/build outcome, new output or a new status from a
//! background run, a first read of a path, a search that found a match, a
//! checklist change, a plan submission — and stops a loop that has gone this
//! many turns without any of them. A successful read, search or inspection that
//! returned nothing new (a zero-match search, a repeated inspection) is idle,
//! and consecutive idle calls are counted against the same limit: the model is
//! noted at the limit and again at twice it, and the job stops at three times
//! it. A silent success from any other shell command (`mkdir`, `chmod`,
//! `git add`/`commit`, an install) is a neutral workspace change, not churn, so
//! it does not build the streak. A silent poll of a live background run is a
//! wait, not a stuck turn: it neither advances the no-progress guard nor counts
//! as a repeat, and past the silent cap it stays a wait up to the job's wall
//! clock (or [`SILENT_WAIT_TOTAL`] when the job has none), so a silent build
//! can be waited on to completion. It also stops a loop that keeps repeating
//! one call with the same result.
//!
//! The classifier is deliberately independent of the loop: it takes plain
//! observations, so its rules are unit-tested on their own and the loop only
//! forwards what each settled tool call did.

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::Value;

mod call_guard;
mod classifier;
mod classifier_keys;
mod handle;
mod inspection;
mod repeat;

pub use classifier::ProgressClassifier;
pub use handle::{ProgressHandle, ProgressObserver, ProgressSnapshot};

/// Consecutive model turns without progress before the job stops.
pub const DEFAULT_NO_PROGRESS_TURNS: u32 = 8;
/// Identical results from one call, in a row, before the job stops.
pub const DEFAULT_REPEAT_LIMIT: u32 = 3;
/// Silent polls of one live background run before the shell's wait starts
/// backing off. Up to this many, a poll waits the model's requested budget;
/// past it the shell waits 120 s, then 240 s, then 300 s per poll.
pub const SILENT_POLL_LIMIT: u32 = 3;

/// The total time silent polls of a live background run may count as waits
/// when the job sets no wall clock. With the shell's backoff this is a handful
/// of bounded waits; past it a silent poll is an idle turn again, so a run
/// that never speaks cannot hold the no-progress guard off forever.
pub const SILENT_WAIT_TOTAL: Duration = Duration::from_secs(1800);

/// The progress guards one loop runs under. Every field is explicit: `None`
/// disables that guard, and the adapters state the resolved defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgressLimits {
    /// Consecutive turns with no progress before stopping; `None` disables.
    pub no_progress_turns: Option<u32>,
    /// Consecutive identical results before stopping; `None` disables.
    pub repeat_limit: Option<u32>,
    /// Wall-clock ceiling from the first turn; `None` disables.
    pub wall_clock: Option<Duration>,
    /// Provider-reported cost ceiling in micro-USD; `None` disables.
    pub max_cost_micros: Option<u64>,
    /// The config prefix the stop names (`limits.job` or `limits.run`).
    pub limit_prefix: &'static str,
}

impl Default for ProgressLimits {
    fn default() -> Self {
        Self {
            no_progress_turns: Some(DEFAULT_NO_PROGRESS_TURNS),
            repeat_limit: Some(DEFAULT_REPEAT_LIMIT),
            wall_clock: None,
            max_cost_micros: None,
            limit_prefix: "limits.job",
        }
    }
}

/// Why a progress guard stopped the loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressStopKind {
    NoProgress,
    Repeating,
    WallClock,
    Cost,
}

impl ProgressStopKind {
    /// The closed `failure.class` value the job records.
    pub fn class(self) -> &'static str {
        match self {
            Self::NoProgress => "no_progress",
            Self::Repeating => "repeating",
            Self::WallClock => "wall_clock",
            Self::Cost => "cost",
        }
    }

    /// A progress stop ends cleanly: the model gets one final turn. The
    /// clock and cost guards are hard stops.
    pub fn is_progress(self) -> bool {
        matches!(self, Self::NoProgress | Self::Repeating)
    }
}

/// One progress guard's stop, with everything a job result must name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressStop {
    pub kind: ProgressStopKind,
    /// The dotted config key to change, e.g. `limits.job.no_progress_turns`.
    pub limit: String,
    pub limit_value: Option<u64>,
    /// What the guard measured when it fired.
    pub measured: u64,
    /// One human line naming the key to change.
    pub message: String,
}

impl std::fmt::Display for ProgressStop {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

/// What one settled tool call showed the classifier.
#[derive(Debug, Clone, Copy)]
pub struct ToolObservation<'a> {
    pub name: &'a str,
    pub arguments: &'a Value,
    /// The call ran (a denial is not an execution).
    pub executed: bool,
    /// The executor reported success.
    pub succeeded: bool,
    pub result: &'a Value,
}

/// What the classifier decided for one observation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Observed {
    /// This call made progress.
    pub progress: bool,
    /// The repeat note to append to the model-facing result.
    pub note: Option<String>,
    /// A repeat stop; the loop stops after the turn's batch settles.
    pub stop: Option<ProgressStop>,
}

/// One turn's report against the no-progress guard: the count it reached, or
/// the reset a turn with progress recorded. A healthy turn with no count to
/// reset reports nothing, so a working run stays quiet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgressCheck {
    /// The turn this check ends.
    pub turn: u64,
    /// The no-progress streak after this turn; for a reset, the streak that
    /// was cleared.
    pub turns_without_progress: u32,
    /// The configured guard, or `None` when it is disabled.
    pub limit: Option<u32>,
    /// This turn made progress and cleared a non-zero streak.
    pub reset: bool,
    /// The last turn that made progress, after this turn's classification.
    pub last_progress_turn: Option<u64>,
}

impl ProgressCheck {
    /// The short line every surface reports. A reset names the cleared streak;
    /// an idle turn names the count against the configured limit.
    pub fn message(&self) -> String {
        if self.reset {
            return format!(
                "progress resumed at turn {}; no-progress count reset (was {})",
                self.turn, self.turns_without_progress
            );
        }
        match self.limit {
            Some(limit) => format!(
                "no progress for {} of {limit} turns",
                self.turns_without_progress
            ),
            None => format!("no progress for {} turns", self.turns_without_progress),
        }
    }
}

/// The progress a job reports: how many turns ran, when progress last
/// happened, how many events of each kind were seen, and the latest check
/// against the no-progress guard.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProgressReport {
    pub turns: u64,
    pub last_progress_turn: Option<u64>,
    pub progress_events: BTreeMap<String, u64>,
    /// The latest turn's check; `None` on a healthy turn that had no streak to
    /// report and on a run whose guard is disabled.
    pub check: Option<ProgressCheck>,
}

#[cfg(test)]
#[path = "progress_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "progress_background_tests.rs"]
mod background_tests;

#[cfg(test)]
#[path = "progress_checks_tests.rs"]
mod checks_tests;

#[cfg(test)]
#[path = "progress_churn_tests.rs"]
mod churn_tests;

#[cfg(test)]
#[path = "progress_inspection_tests.rs"]
mod inspection_tests;
