use std::collections::BTreeMap;

use saya_types::{Budgets, MAX_BUDGET_ENDPOINTS, is_name_shaped};

use super::dimension::parse_count;
use super::{BudgetLimit, RunLimitsFile};
use crate::ConfigError;
use crate::jobs::require_at_least_one;

/// The run surface's resolved ceilings — what the engine layers RunSpec and
/// step budgets over, per dimension. Every dimension is unlimited until a
/// layer states one: that is the rule the four `[jobs]` ceilings follow, and
/// `[limits.run]` keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRunLimits {
    /// Wall-clock ceiling for a run, in seconds. `Unlimited` until declared —
    /// the engine pauses on a declared ceiling, never overruns.
    pub wall_clock_seconds: BudgetLimit,
    /// Turn ceiling across the run's episodes.
    pub turns: BudgetLimit,
    /// Ceiling on total tool calls across the run's episodes.
    pub tool_calls: BudgetLimit,
    /// Per-endpoint token ceilings, keyed by run-scoped endpoint name, the
    /// same shape the run contracts carry. Empty when nothing is declared.
    pub tokens_per_endpoint: BTreeMap<String, u64>,
    /// Consecutive model turns without progress before a step stops. Zero
    /// disables the guard.
    pub no_progress_turns: u32,
    /// Identical results from one call, in a row, before a step stops. Zero
    /// disables the guard.
    pub repeat_limit: u32,
}

impl Default for ResolvedRunLimits {
    /// A run with nothing declared has no ceiling.
    fn default() -> Self {
        Self {
            wall_clock_seconds: BudgetLimit::Unlimited,
            turns: BudgetLimit::Unlimited,
            tool_calls: BudgetLimit::Unlimited,
            tokens_per_endpoint: BTreeMap::new(),
            no_progress_turns: DEFAULT_NO_PROGRESS_TURNS,
            repeat_limit: DEFAULT_REPEAT_LIMIT,
        }
    }
}

impl ResolvedRunLimits {
    /// The resolved ceilings as the run contract's budget shape. The result
    /// always satisfies the contract's own validation.
    pub fn budgets(&self) -> Budgets {
        let mut budgets = Budgets::default();
        budgets.wall_clock = self.wall_clock_seconds.duration();
        budgets.turns = self.turns.value();
        budgets.tool_calls = self.tool_calls.value();
        budgets.tokens_per_endpoint = self.tokens_per_endpoint.clone();
        // Zero disables a progress guard; the contract spells that as an
        // unset dimension, so the engine never sees a zero to interpret.
        budgets.no_progress_turns =
            (self.no_progress_turns > 0).then(|| u64::from(self.no_progress_turns));
        budgets.repeat_limit = (self.repeat_limit > 0).then(|| u64::from(self.repeat_limit));
        budgets
    }
}

/// Resolves `[limits.run]`. Each key is spelled out rather than assembled from
/// the sub-table name so an invalid value names the exact config key to fix.
pub(crate) fn resolve(file: &RunLimitsFile) -> Result<ResolvedRunLimits, ConfigError> {
    let defaults = ResolvedRunLimits::default();
    Ok(ResolvedRunLimits {
        wall_clock_seconds: BudgetLimit::parse(
            file.wall_clock_seconds.as_ref(),
            "limits.run.wall_clock_seconds",
            defaults.wall_clock_seconds,
        )?,
        turns: BudgetLimit::parse(file.turns.as_ref(), "limits.run.turns", defaults.turns)?,
        tool_calls: BudgetLimit::parse(
            file.tool_calls.as_ref(),
            "limits.run.tool_calls",
            defaults.tool_calls,
        )?,
        tokens_per_endpoint: match &file.tokens_per_endpoint {
            Some(map) => resolve_token_map(map)?,
            None => BTreeMap::new(),
        },
        no_progress_turns: parse_count(
            file.no_progress_turns.as_ref(),
            "limits.run.no_progress_turns",
            defaults.no_progress_turns,
            MAX_NO_PROGRESS_TURNS,
        )?,
        repeat_limit: parse_count(
            file.repeat_limit.as_ref(),
            "limits.run.repeat_limit",
            defaults.repeat_limit,
            MAX_REPEAT_LIMIT,
        )?,
    })
}

const DEFAULT_NO_PROGRESS_TURNS: u32 = 8;
const DEFAULT_REPEAT_LIMIT: u32 = 3;
const MAX_NO_PROGRESS_TURNS: u32 = 10_000;
const MAX_REPEAT_LIMIT: u32 = 10_000;

/// Validates the token map against the shape the run contracts carry, so a
/// config the contract would reject at plan-validation time is caught at
/// resolve time instead. The endpoint-count bound is the contract's own
/// (`MAX_BUDGET_ENDPOINTS`), and keys must have the run-scoped name shape.
///
/// A value is a plain positive integer: an endpoint left out has no ceiling,
/// so this map needs no `unlimited` spelling, and a zero would starve the
/// endpoint on its first call — a typo, not an intent.
fn resolve_token_map(map: &BTreeMap<String, u64>) -> Result<BTreeMap<String, u64>, ConfigError> {
    if map.len() > MAX_BUDGET_ENDPOINTS {
        return Err(ConfigError::SettingAboveMaximum {
            field: TOKENS_PER_ENDPOINT,
            value: map.len(),
            max: MAX_BUDGET_ENDPOINTS,
        });
    }
    for (endpoint, tokens) in map {
        if !is_name_shaped(endpoint) {
            return Err(ConfigError::InvalidEndpointName {
                field: TOKENS_PER_ENDPOINT,
                key: endpoint.clone(),
            });
        }
        require_at_least_one(TOKENS_PER_ENDPOINT, *tokens)?;
    }
    Ok(map.clone())
}

/// The dotted key every token-map refusal names, so the fix points at the
/// table rather than at the endpoint inside it.
const TOKENS_PER_ENDPOINT: &str = "limits.run.tokens_per_endpoint";
