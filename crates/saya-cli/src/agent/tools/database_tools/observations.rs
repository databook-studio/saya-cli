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

    /// Reports whether a succeeded observation this turn touched the given
    /// qualified object. A read-only, non-consuming read: `contract_propose` calls
    /// it *during* the turn (before the application drains) to pick an evidence
    /// kind, so it must not clear the log.
    ///
    /// Matching is case-insensitive on the trailing parts the observation
    /// actually named: a proposal for `analytics.public.orders` matches an
    /// observation of `orders`, `public.orders`, or `analytics.public.orders`.
    /// Only a `Succeeded` observation is evidence a proposal can lean on — a
    /// failed or denied query touched nothing it can claim.
    pub(crate) fn touched(&self, catalog: &str, schema: &str, object: &str) -> bool {
        let guard = self
            .observations
            .lock()
            .expect("observation log not poisoned");
        let proposed = [catalog, schema, object];
        guard.iter().any(|obs| {
            obs.outcome == ObservationOutcome::Succeeded
                && obs.objects.iter().any(|path| path_matches(path, &proposed))
        })
    }
}

/// True when the observed `path` (1–3 parts, as written) names the same object
/// as the proposed `[catalog, schema, object]`, anchored at the trailing parts
/// and compared case-insensitively.
fn path_matches(path: &[String], proposed: &[&str; 3]) -> bool {
    let n = path.len().min(3);
    if n == 0 {
        return false;
    }
    // Compare the trailing `n` parts: path[-n..] vs proposed[3-n..].
    path.iter()
        .skip(path.len() - n)
        .zip(proposed.iter().skip(3 - n))
        .all(|(obs, want)| obs.eq_ignore_ascii_case(want))
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

    fn obs_with_objects(tool: &str, objects: &[&[&str]]) -> ToolObservation {
        ToolObservation {
            objects: objects
                .iter()
                .map(|parts| parts.iter().map(|s| s.to_string()).collect())
                .collect(),
            ..obs(tool)
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

    #[test]
    fn touched_matches_underspecified_observations_anchored_at_trailing_parts() {
        let log = ObservationLog::new();
        // A 3-part observation pins its catalog: the same 3-part proposal matches,
        // but a different-catalog proposal for the same schema+object does not —
        // the query touched `analytics.public.orders`, never `cat.public.orders`.
        log.record(obs_with_objects(
            "bounded_sql_query",
            &[&["analytics", "public", "orders"]],
        ));
        assert!(log.touched("analytics", "public", "orders"));
        assert!(!log.touched("cat", "public", "orders"));

        // An under-specified observation (fewer parts) matches any proposal that
        // agrees on the trailing parts the observation actually named.
        let log = ObservationLog::new();
        log.record(obs_with_objects(
            "bounded_sql_query",
            &[&["public", "orders"]],
        ));
        assert!(log.touched("analytics", "public", "orders"));
        assert!(log.touched("cat", "public", "orders"));

        let log = ObservationLog::new();
        log.record(obs_with_objects("bounded_sql_query", &[&["orders"]]));
        assert!(log.touched("analytics", "public", "orders"));
        assert!(log.touched("cat", "schema", "orders"));
    }

    #[test]
    fn touched_misses_a_different_object() {
        let log = ObservationLog::new();
        log.record(obs_with_objects(
            "bounded_sql_query",
            &[&["public", "orders"]],
        ));
        assert!(!log.touched("analytics", "public", "lineitems"));
        assert!(!log.touched("analytics", "staging", "orders"));
    }

    #[test]
    fn touched_ignores_failed_and_denied_observations() {
        let log = ObservationLog::new();
        let mut failed = obs_with_objects("bounded_sql_query", &[&["public", "orders"]]);
        failed.outcome = ObservationOutcome::Failed;
        log.record(failed);
        let mut denied = obs_with_objects("bounded_sql_query", &[&["public", "orders"]]);
        denied.outcome = ObservationOutcome::Denied;
        log.record(denied);
        assert!(
            !log.touched("analytics", "public", "orders"),
            "a failed or denied query touched nothing a proposal can lean on"
        );
    }

    #[test]
    fn touched_is_non_consuming() {
        let log = ObservationLog::new();
        log.record(obs_with_objects(
            "bounded_sql_query",
            &[&["public", "orders"]],
        ));
        assert!(log.touched("cat", "public", "orders"));
        // The log is unchanged: a second read still sees the observation, and a
        // later drain returns it. `contract_propose` reads mid-turn.
        assert!(log.touched("cat", "public", "orders"));
        assert_eq!(log.drain().observations.len(), 1);
    }
}
