//! Request-scoped collection of what a tool call touched, for Phase 3c to turn
//! into candidate proposals. This slice COLLECTS ONLY — `record` stores an
//! observation and `drain` returns it; nothing is persisted.
//!
//! An observation records tool name, outcome, profile, object and column
//! references, row count and truncation, and **nothing else**. No SQL, no
//! result values, no error strings: a connector error message can contain a
//! value from the query, which is why the outcome is an enum and not a message.
//! See `.claude/specs/spec-3b2-observation-collector.md`.

use saya_types::ProfileIdentity;
use std::sync::Mutex;

/// The bound on observations per turn. Further records are dropped and `drain`
/// reports that it truncated.
const MAX_OBSERVATIONS: usize = 32;

/// What a tool call contributed to the turn's evidence. Exhaustive: the fields
/// here are the whole of what Phase 3 may record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolObservation {
    pub(crate) tool: String,
    pub(crate) outcome: ObservationOutcome,
    pub(crate) profile: Option<ProfileIdentity>,
    /// Fully qualified objects the statement touched, as written.
    pub(crate) objects: Vec<Vec<String>>,
    pub(crate) columns: Vec<String>,
    pub(crate) row_count: Option<usize>,
    pub(crate) truncated: Option<bool>,
    /// True when extraction could not model part of the statement.
    pub(crate) references_partial: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum ObservationOutcome {
    Succeeded,
    Failed,
    Denied,
}

/// The drained log plus whether the 32-observation cap dropped records.
//
// Unused in the production lib today: Phase 3c is the first caller, draining
// the log from the application operation after a turn. `expect` documents that
// this is intentional and will flag the attribute if a caller appears.
#[cfg_attr(not(test), expect(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DrainedObservations {
    pub(crate) observations: Vec<ToolObservation>,
    pub(crate) truncated: bool,
}

/// A request-scoped, bounded log of tool observations.
///
/// `std::sync::Mutex` (not `RefCell` or a channel) because `DatabaseTools` is
/// shared by `&self` across the agent loop's async fan-out, whose concurrent
/// futures all borrow it — `RefCell` is not `Sync`, and a channel is overkill
/// for an append-then-drain buffer with one reader.
pub(crate) struct ObservationLog {
    observations: Mutex<Vec<ToolObservation>>,
}

impl ObservationLog {
    pub(crate) fn new() -> Self {
        Self {
            observations: Mutex::new(Vec::new()),
        }
    }

    /// Appends an observation, dropping it once 32 are held this turn.
    pub(crate) fn record(&self, observation: ToolObservation) {
        let mut guard = self
            .observations
            .lock()
            .expect("observation log not poisoned");
        if guard.len() < MAX_OBSERVATIONS {
            guard.push(observation);
        }
    }

    /// Returns and clears the turn's observations, reporting whether the cap
    /// dropped any. A second call returns nothing — a turn cannot double-count.
    //
    // Unused in the production lib today: Phase 3c drains from the application
    // operation. See `DrainedObservations` for the `expect` rationale.
    #[cfg_attr(not(test), expect(dead_code))]
    pub(crate) fn drain(&self) -> DrainedObservations {
        let mut guard = self
            .observations
            .lock()
            .expect("observation log not poisoned");
        let observations = std::mem::take(&mut *guard);
        DrainedObservations {
            truncated: observations.len() == MAX_OBSERVATIONS,
            observations,
        }
    }
}

impl Default for ObservationLog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(tool: &str) -> ToolObservation {
        ToolObservation {
            tool: tool.into(),
            outcome: ObservationOutcome::Succeeded,
            profile: None,
            objects: Vec::new(),
            columns: Vec::new(),
            row_count: None,
            truncated: None,
            references_partial: false,
        }
    }

    #[test]
    fn record_then_drain_returns_in_order_and_empties() {
        let log = ObservationLog::new();
        log.record(obs("a"));
        log.record(obs("b"));
        let drained = log.drain();
        assert_eq!(
            drained
                .observations
                .iter()
                .map(|o| &o.tool)
                .collect::<Vec<_>>(),
            &["a", "b"]
        );
        assert!(!drained.truncated);
        // A second drain is empty: a turn cannot double-count.
        let again = log.drain();
        assert!(again.observations.is_empty());
        assert!(!again.truncated);
    }

    #[test]
    fn cap_holds_and_drain_reports_truncation() {
        let log = ObservationLog::new();
        for i in 0..40 {
            log.record(obs(&format!("t{i}")));
        }
        let drained = log.drain();
        assert_eq!(drained.observations.len(), MAX_OBSERVATIONS);
        assert_eq!(drained.observations.last().unwrap().tool, "t31");
        assert!(drained.truncated);
    }
}
