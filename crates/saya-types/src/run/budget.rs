//! Budget ceilings for a run and its steps.
//!
//! A ceiling left unset is unlimited at the contract level — the engine
//! enforces what is declared, pausing (never silently overrunning) when a
//! declared budget trips. Usage is reported separately, by the run's events;
//! an unreported figure means "unknown", never zero.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::RunContractError;
use super::scope::is_name_shaped;

/// How many endpoints a budget may carry token ceilings for. Endpoint names
/// are run-scoped names, so the same small bound applies.
pub const MAX_BUDGET_ENDPOINTS: usize = 8;

/// The ceilings a run (or one of its steps) is declared with. `Default` is
/// "nothing declared" — every dimension unlimited until the composition root
/// or the plan sets it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct Budgets {
    /// Wall-clock ceiling. The engine owns the clock and checks it per tick —
    /// each agent event emission is one tick (`EngineEventSink::tick` in
    /// `saya-harness`); there is no additional episode-end check. It is the
    /// honest backstop where a provider reports no usage.
    pub wall_clock: Option<Duration>,
    /// Token ceilings keyed by endpoint name, as bound in the run's endpoint
    /// bindings. An endpoint absent from the map has no ceiling.
    pub tokens_per_endpoint: BTreeMap<String, u64>,
    /// Bytes the run may download (fetch bound).
    pub downloaded_bytes: Option<u64>,
    /// Total bytes the workspace may hold (workspace bound).
    pub workspace_bytes: Option<u64>,
    /// File count the workspace may hold (workspace bound).
    pub workspace_files: Option<u64>,
    /// How many child processes the runner may start (runner bound).
    pub process_count: Option<u64>,
    /// Per-process wall-clock ceiling (runner bound).
    pub process_time: Option<Duration>,
    /// Turns across the run's episodes.
    pub turns: Option<u64>,
    /// Tool calls across the run's episodes.
    pub tool_calls: Option<u64>,
}

impl Budgets {
    /// Shape check for the map: endpoint keys must be run-scoped names and
    /// the map bounded. Steps arriving through deserialization skipped the
    /// validating constructors, so the plan validator re-runs this.
    pub fn validate(&self) -> Result<(), RunContractError> {
        if self.tokens_per_endpoint.len() > MAX_BUDGET_ENDPOINTS {
            return Err(RunContractError::TooManyBudgetEndpoints);
        }
        if !self.tokens_per_endpoint.keys().all(|k| is_name_shaped(k)) {
            return Err(RunContractError::InvalidEndpointName);
        }
        Ok(())
    }

    /// True when nothing `self` asks for exceeds `ceiling`. A dimension unset
    /// on the ceiling is unlimited; a dimension unset on `self` asks for
    /// nothing. Token ceilings are per endpoint: an endpoint the ceiling does
    /// not name is unlimited for that endpoint.
    pub fn is_within(&self, ceiling: &Budgets) -> bool {
        let count_within = |asked: Option<u64>, cap: Option<u64>| match (asked, cap) {
            (Some(asked), Some(cap)) => asked <= cap,
            _ => true,
        };
        let time_within = |asked: Option<Duration>, cap: Option<Duration>| match (asked, cap) {
            (Some(asked), Some(cap)) => asked <= cap,
            _ => true,
        };
        time_within(self.wall_clock, ceiling.wall_clock)
            && count_within(self.downloaded_bytes, ceiling.downloaded_bytes)
            && count_within(self.workspace_bytes, ceiling.workspace_bytes)
            && count_within(self.workspace_files, ceiling.workspace_files)
            && count_within(self.process_count, ceiling.process_count)
            && time_within(self.process_time, ceiling.process_time)
            && count_within(self.turns, ceiling.turns)
            && count_within(self.tool_calls, ceiling.tool_calls)
            && self.tokens_per_endpoint.iter().all(|(endpoint, asked)| {
                ceiling
                    .tokens_per_endpoint
                    .get(endpoint)
                    .is_none_or(|&cap| *asked <= cap)
            })
    }
}
