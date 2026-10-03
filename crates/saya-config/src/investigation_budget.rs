use std::num::NonZeroU64;

use crate::{ConfigError, model::AiFile};

/// A validated ceiling for one interactive investigation dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetLimit {
    Finite(NonZeroU64),
    Unlimited,
}

/// The validated interactive investigation ceilings from `[ai]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedInvestigationBudgets {
    pub logical_answering_requests: BudgetLimit,
    pub requested_tool_calls: BudgetLimit,
    pub elapsed_seconds: BudgetLimit,
    pub known_reported_tokens: BudgetLimit,
}

impl Default for ResolvedInvestigationBudgets {
    fn default() -> Self {
        Self {
            logical_answering_requests: finite_default(24),
            requested_tool_calls: finite_default(64),
            elapsed_seconds: finite_default(300),
            known_reported_tokens: finite_default(250_000),
        }
    }
}

pub(crate) fn resolve(file: &AiFile) -> Result<ResolvedInvestigationBudgets, ConfigError> {
    let defaults = ResolvedInvestigationBudgets::default();
    Ok(ResolvedInvestigationBudgets {
        logical_answering_requests: resolve_one(
            file.investigation_max_logical_answering_requests.as_ref(),
            "investigation_max_logical_answering_requests",
            defaults.logical_answering_requests,
        )?,
        requested_tool_calls: resolve_one(
            file.investigation_max_requested_tool_calls.as_ref(),
            "investigation_max_requested_tool_calls",
            defaults.requested_tool_calls,
        )?,
        elapsed_seconds: resolve_one(
            file.investigation_max_elapsed_seconds.as_ref(),
            "investigation_max_elapsed_seconds",
            defaults.elapsed_seconds,
        )?,
        known_reported_tokens: resolve_one(
            file.investigation_max_known_reported_tokens.as_ref(),
            "investigation_max_known_reported_tokens",
            defaults.known_reported_tokens,
        )?,
    })
}

fn resolve_one(
    value: Option<&toml::Value>,
    field: &'static str,
    default: BudgetLimit,
) -> Result<BudgetLimit, ConfigError> {
    match value {
        None => Ok(default),
        Some(toml::Value::String(value)) if value == "unlimited" => Ok(BudgetLimit::Unlimited),
        Some(toml::Value::Integer(value)) => u64::try_from(*value)
            .ok()
            .and_then(NonZeroU64::new)
            .map(BudgetLimit::Finite)
            .ok_or(ConfigError::InvalidInvestigationBudget { field }),
        Some(_) => Err(ConfigError::InvalidInvestigationBudget { field }),
    }
}

fn finite_default(value: u64) -> BudgetLimit {
    BudgetLimit::Finite(NonZeroU64::new(value).expect("built-in investigation budgets are nonzero"))
}
