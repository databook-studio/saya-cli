//! The one classifier: per-turn progress, the repeat detector, the call-level
//! churn guard, and the two hard guards. It owns no I/O and no async, so every
//! rule is unit-tested.

use std::collections::{BTreeMap, HashMap};
use std::time::Instant;

use super::call_guard::{CallGuard, CallOutcome};
use super::classifier_keys::repeat_key;
use super::repeat::{RepeatDetector, RepeatOutcome};
use super::{
    ProgressCheck, ProgressLimits, ProgressReport, ProgressStop, ProgressStopKind, ToolObservation,
};
use crate::loop_runner::failure_key::{canonical_json, fnv1a64};

mod classify;

/// What one observation showed the turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Classification {
    /// The call moved the work forward.
    Progress,
    /// Neither progress nor churn: a wait on a live background run that showed
    /// nothing new, or a silent success from a mutating shell command. Not a
    /// stuck turn either.
    Neutral,
    /// Nothing moved.
    Idle,
}

#[derive(Debug)]
pub struct ProgressClassifier {
    limits: ProgressLimits,
    /// The last result digest of each read key, so a repeat of the same range
    /// is idle and changed bytes are progress.
    read_results: HashMap<String, u64>,
    /// The last result digest of each search key, so a repeat is idle and a
    /// changed result is progress.
    search_results: HashMap<String, u64>,
    /// The last result digest of each shell inspection target, so paging a
    /// file or re-tailing a growing log is progress while the same look at the
    /// same bytes is not.
    inspection_results: HashMap<String, u64>,
    shell_outcomes: HashMap<String, (i64, u64)>,
    /// The last status a background id reported, so a poll can tell a status
    /// change from a repeated wait.
    background_status: HashMap<u64, String>,
    /// Silent polls of each live background id. Up to
    /// [`SILENT_POLL_LIMIT`](super::SILENT_POLL_LIMIT) they are waits; past it
    /// they are idle turns.
    silent_polls: HashMap<u64, u32>,
    last_checklist_digest: Option<u64>,
    repeat: RepeatDetector,
    call_guard: CallGuard,
    turns: u64,
    last_progress_turn: Option<u64>,
    turns_without_progress: u32,
    turn_had_progress: bool,
    /// The turn only waited on a live background run: it neither clears nor
    /// advances the no-progress streak.
    turn_was_neutral: bool,
    last_check: Option<ProgressCheck>,
    progress_events: BTreeMap<String, u64>,
    cost_micros: u64,
    last_reported_cost_micros: u64,
    pending_stop: Option<ProgressStop>,
    /// A guard fired: the remaining turns are the clean wind-down, so they do
    /// not move the check the stop reported.
    stopped: bool,
    started: Instant,
}

impl ProgressClassifier {
    pub fn new(limits: ProgressLimits) -> Self {
        Self {
            limits,
            read_results: HashMap::new(),
            search_results: HashMap::new(),
            inspection_results: HashMap::new(),
            shell_outcomes: HashMap::new(),
            background_status: HashMap::new(),
            silent_polls: HashMap::new(),
            last_checklist_digest: None,
            repeat: RepeatDetector::default(),
            call_guard: CallGuard::default(),
            turns: 0,
            last_progress_turn: None,
            turns_without_progress: 0,
            turn_had_progress: false,
            turn_was_neutral: false,
            last_check: None,
            progress_events: BTreeMap::new(),
            cost_micros: 0,
            last_reported_cost_micros: 0,
            pending_stop: None,
            stopped: false,
            started: Instant::now(),
        }
    }

    /// What remains of the job's wall-clock ceiling, or `None` when unbounded.
    pub fn remaining_wall(&self) -> Option<std::time::Duration> {
        let wall = self.limits.wall_clock;
        wall.map(|limit| limit.saturating_sub(self.started.elapsed()))
    }

    /// Classifies one settled call and records what it changed.
    pub fn observe(&mut self, observation: ToolObservation<'_>) -> super::Observed {
        if !observation.executed {
            return super::Observed::default();
        }
        let result_digest = fnv1a64(canonical_json(observation.result).as_bytes());
        let key = repeat_key(observation.name, observation.arguments);
        let classification = self.classify(observation, result_digest);
        let mut observed = super::Observed {
            progress: classification == Classification::Progress,
            ..super::Observed::default()
        };
        if observed.progress {
            self.turn_had_progress = true;
            *self
                .progress_events
                .entry(self.event_kind(observation).to_owned())
                .or_insert(0) += 1;
            self.call_guard.progress();
        }
        if classification == Classification::Neutral {
            self.turn_was_neutral = true;
        }
        // An unchanged poll of a live run is a wait, not a loop: it must not
        // feed the repeat detector. Every other call still does.
        if classification != Classification::Neutral
            && let Some(limit) = self.limits.repeat_limit.filter(|limit| *limit > 0)
        {
            match self.repeat.observe(key, result_digest, limit) {
                RepeatOutcome::None => {}
                RepeatOutcome::Note => {
                    observed.note = Some(format!(
                        "You are repeating {} with the same result; change approach or finish.",
                        observation.name
                    ));
                }
                RepeatOutcome::Stop => {
                    let stop = ProgressStop {
                        kind: ProgressStopKind::Repeating,
                        limit: format!("{}.repeat_limit", self.limits.limit_prefix),
                        limit_value: Some(u64::from(limit)),
                        measured: u64::from(limit).saturating_add(1),
                        message: format!(
                            "stopped: {} returned the same result {} times in a row; change approach or raise {}.repeat_limit",
                            observation.name,
                            u64::from(limit).saturating_add(1),
                            self.limits.limit_prefix
                        ),
                    };
                    self.pending_stop = Some(stop.clone());
                    observed.stop = Some(stop);
                }
            }
        }
        // Consecutive calls that moved nothing trip the call-level guard even
        // when a turn around them happened to make progress.
        if classification == Classification::Idle
            && observed.stop.is_none()
            && self.pending_stop.is_none()
            && !self.stopped
        {
            match self
                .call_guard
                .idle(self.limits.no_progress_turns, self.limits.limit_prefix)
            {
                CallOutcome::None => {}
                CallOutcome::Note {
                    count,
                    stop_at,
                    last,
                } => {
                    observed
                        .note
                        .get_or_insert_with(|| call_note(count, stop_at, last));
                }
                CallOutcome::Stop(stop) => {
                    self.pending_stop = Some(stop.clone());
                    observed.stop = Some(stop);
                }
            }
        }
        observed
    }

    /// Takes the stop a call in the last settled batch fired, if any: the
    /// repeat detector's or the call-level churn guard's.
    pub fn take_stop(&mut self) -> Option<ProgressStop> {
        let stop = self.pending_stop.take();
        self.stopped |= stop.is_some();
        stop
    }

    /// Ends one model turn and returns a hard stop or the no-progress stop
    /// when a guard fires. `cost_micros` is the run's cumulative
    /// provider-reported cost, if any.
    pub fn finish_turn(&mut self, cost_micros: Option<u64>) -> Option<ProgressStop> {
        self.turns = self.turns.saturating_add(1);
        if self.stopped {
            // The guard already fired; the wind-down turns are not a new
            // measurement, so the stop's check stays the one reported.
            self.turn_had_progress = false;
            self.turn_was_neutral = false;
            return None;
        }
        let had_progress = self.turn_had_progress;
        let neutral = self.turn_was_neutral && !had_progress;
        let previous_without_progress = self.turns_without_progress;
        if had_progress {
            self.last_progress_turn = Some(self.turns);
            self.turns_without_progress = 0;
        } else if !neutral {
            self.turns_without_progress = self.turns_without_progress.saturating_add(1);
        }
        self.turn_had_progress = false;
        self.turn_was_neutral = false;
        let limit = self.limits.no_progress_turns.filter(|limit| *limit > 0);
        // A healthy turn with no streak to clear stays silent, and so does a
        // turn that only waited on a live background run: neither is a
        // measurement against the guard. An idle turn records its count
        // against the configured limit, and a turn that clears a non-zero
        // streak records the reset once.
        self.last_check = match (had_progress, limit) {
            (true, Some(limit)) if previous_without_progress > 0 => Some(ProgressCheck {
                turn: self.turns,
                turns_without_progress: previous_without_progress,
                limit: Some(limit),
                reset: true,
                last_progress_turn: self.last_progress_turn,
            }),
            (false, Some(limit)) if !neutral => Some(ProgressCheck {
                turn: self.turns,
                turns_without_progress: self.turns_without_progress,
                limit: Some(limit),
                reset: false,
                last_progress_turn: self.last_progress_turn,
            }),
            _ => None,
        };

        if let Some(reported) = cost_micros {
            let delta = reported.saturating_sub(self.last_reported_cost_micros);
            self.cost_micros = self.cost_micros.saturating_add(delta);
            self.last_reported_cost_micros = reported;
        }
        if let Some(max) = self.limits.max_cost_micros
            && self.cost_micros > max
        {
            self.stopped = true;
            return Some(ProgressStop {
                kind: ProgressStopKind::Cost,
                limit: format!("{}.max_cost_usd", self.limits.limit_prefix),
                limit_value: Some(max),
                measured: self.cost_micros,
                message: format!(
                    "stopped: provider-reported cost reached {} micro-USD, above {}.max_cost_usd; raise it or finish the job",
                    self.cost_micros, self.limits.limit_prefix
                ),
            });
        }
        if let Some(limit) = self.limits.wall_clock
            && self.started.elapsed() >= limit
        {
            self.stopped = true;
            return Some(ProgressStop {
                kind: ProgressStopKind::WallClock,
                limit: format!("{}.wall_clock_seconds", self.limits.limit_prefix),
                limit_value: Some(limit.as_secs()),
                measured: self.started.elapsed().as_secs(),
                message: format!(
                    "stopped: the job ran {}s, at its {}.wall_clock_seconds ceiling; raise it or finish the job",
                    self.started.elapsed().as_secs(),
                    self.limits.limit_prefix
                ),
            });
        }
        let limit = self.limits.no_progress_turns.filter(|limit| *limit > 0)?;
        let stop = (self.turns_without_progress >= limit).then(|| ProgressStop {
            kind: ProgressStopKind::NoProgress,
            limit: format!("{}.no_progress_turns", self.limits.limit_prefix),
            limit_value: Some(u64::from(limit)),
            measured: u64::from(self.turns_without_progress),
            message: format!(
                "stopped: no progress for {} of {} turns ({}); resume with `saya job resume <job>` or raise {}.no_progress_turns",
                self.turns_without_progress,
                limit,
                match self.last_progress_turn {
                    Some(turn) => format!("last progress at turn {turn}"),
                    None => "no progress turn recorded".to_owned(),
                },
                self.limits.limit_prefix
            ),
        });
        self.stopped = stop.is_some();
        stop
    }

    /// The durable summary a job reports.
    pub fn report(&self) -> ProgressReport {
        ProgressReport {
            turns: self.turns,
            last_progress_turn: self.last_progress_turn,
            progress_events: self.progress_events.clone(),
            check: self.last_check,
        }
    }
}

/// The one short note a call-guard threshold sends the model. The second note
/// names the stop so the model knows how much room is left.
fn call_note(count: u32, stop_at: u32, last: bool) -> String {
    if last {
        format!(
            "Still no material progress after {count} calls; the job stops at {stop_at} calls without a material change."
        )
    } else {
        format!("No material progress in the last {count} calls; change approach or finish.")
    }
}
