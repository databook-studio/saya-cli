//! Reading the journal back: whole-line parsing that ignores a torn tail and
//! fails closed on complete-line corruption, and the replay that turns the
//! event order into the state a resume starts from.

use std::{collections::BTreeMap, fs};

use saya_types::RunEvent;

use crate::{HarnessError, io_error};

use super::Journal;

impl Journal {
    /// Every event in the journal, in write order. An absent journal is an
    /// empty one.
    pub fn read(&self) -> Result<Vec<RunEvent>, HarnessError> {
        let raw = match fs::read_to_string(&self.path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(io_error("read run journal", &self.path, error)),
        };
        parse_lines(&raw).map_err(|line| HarnessError::JournalCorrupt { line })
    }

    /// The state a resume starts from, replayed from the journal.
    pub fn rebuild(&self) -> Result<JournalState, HarnessError> {
        Ok(replay(&self.read()?))
    }
}

/// The state a resume starts from, rebuilt by replaying the journal in
/// write order. Steps the plan bound but the journal never mentions are
/// absent here — the engine holds the plan, so "first incomplete step"
/// stays its question to ask.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct JournalState {
    /// A `RunStarted` is recorded.
    pub started: bool,
    /// A `PlanApproved` is recorded.
    pub plan_approved: bool,
    /// Every step the journal mentions, keyed by its plan index. Last write
    /// wins: a step that failed and was started again by the bounded retry
    /// reads back as [`StepState::Started`].
    pub steps: BTreeMap<usize, StepState>,
    /// The journal's last lifecycle event — a pause's reason or a terminal
    /// state lives here. Usage reports and download levels are deliberately
    /// not lifecycle.
    pub last: Option<RunEvent>,
}

/// Where a step stands, as the journal last recorded it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepState {
    /// Its episode began; it may be mid-flight or awaiting a bounded retry.
    Started,
    Completed,
    Failed,
}

/// Replays journal events into the state a resume starts from.
pub fn replay(events: &[RunEvent]) -> JournalState {
    let mut state = JournalState::default();
    for event in events {
        // Every lifecycle event is a candidate for `last`, not only the ones
        // without a state-tracking arm of their own. Setting it inside the
        // catch-all meant a journal ending on StepStarted reported no last
        // event at all — precisely the shape a resume reads. Usage reports
        // and download levels are not lifecycle and never stand in for one.
        if !matches!(
            event,
            RunEvent::Usage { .. }
                | RunEvent::DownloadedBytes { .. }
                | RunEvent::WallClockObserved { .. }
        ) {
            state.last = Some(event.clone());
        }
        match event {
            RunEvent::RunStarted => state.started = true,
            RunEvent::PlanApproved { scopes: _ } => state.plan_approved = true,
            RunEvent::StepStarted { step } => {
                state.steps.insert(*step, StepState::Started);
            }
            RunEvent::StepCompleted { step } => {
                state.steps.insert(*step, StepState::Completed);
            }
            RunEvent::StepFailed { step } => {
                state.steps.insert(*step, StepState::Failed);
            }
            // The deliverables manifest is not lifecycle and not step state:
            // the `StepCompleted` recorded right after it carries the state.
            // A journal ending here reads as a step still in flight.
            RunEvent::Deliverables { .. } => {}
            RunEvent::Usage { .. } => {}
            // The download level is the wallet's spend, not lifecycle and
            // not step state — it is the figure a resume seeds the wallet
            // from, read separately from the replay.
            RunEvent::DownloadedBytes { .. } => {}
            // Clock observations carry accounting authority only; they do
            // not stand in for the latest lifecycle event.
            RunEvent::WallClockObserved { .. } => {}
            _ => {}
        }
    }
    state
}

/// Parses every newline-terminated line. A torn final line — no newline —
/// was never a complete event and is ignored; a complete line that does not
/// parse fails closed with its 1-based line number.
fn parse_lines(raw: &str) -> Result<Vec<RunEvent>, usize> {
    let torn_tail = !raw.is_empty() && !raw.ends_with('\n');
    let total = raw.lines().count();
    let mut events = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        if line.is_empty() || (torn_tail && index + 1 == total) {
            continue;
        }
        events.push(serde_json::from_str(line).map_err(|_| index + 1)?);
    }
    Ok(events)
}
