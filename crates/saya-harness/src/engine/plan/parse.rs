//! The defensive parse of a planner response into a [`RunPlan`].
//!
//! A provider that cannot honour the JSON-mode hint may return prose or a
//! fenced answer; a gateway may truncate mid-object. Every non-plan shape is
//! an ordinary typed outcome — never a panic — and the proposal loop feeds
//! the classification back to the model as its refusal.

use saya_types::RunPlan;
use thiserror::Error;

/// How a planner response failed to be a plan. Ordinary outcomes of a model
/// that returned prose, was truncated, or ignored the schema — never a panic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum PlanParseFailure {
    /// The answer is not a JSON object at all.
    #[error("prose, not a JSON object")]
    Prose,
    /// The JSON object is cut off mid-parse.
    #[error("truncated JSON")]
    Truncated,
    /// The JSON parses but is not a plan.
    #[error("JSON of the wrong shape")]
    WrongShape,
}

/// Parses the planner's answer into a candidate plan. The plan returned is
/// unvalidated — [`RunPlan::validate`] is the gate, called by the driver.
pub(super) fn parse_plan(raw: &str) -> Result<RunPlan, PlanParseFailure> {
    let text = strip_fences(raw).trim();
    if !text.starts_with('{') {
        return Err(PlanParseFailure::Prose);
    }
    match serde_json::from_str::<RunPlan>(text) {
        Ok(plan) => Ok(plan),
        // An object cut off mid-parse is truncation, not the wrong shape.
        Err(error) if error.is_eof() => Err(PlanParseFailure::Truncated),
        Err(_) => Err(PlanParseFailure::WrongShape),
    }
}

/// Strips markdown fences from a model answer: bare JSON passes through, the
/// ```` ```json ```` and bare ```` ``` ```` fences collapse to their inner
/// JSON — the three shapes a provider that ignored the JSON hint can still
/// return. A local copy of the extraction parser's stripper: that one lives
/// in `saya-cli`, and presentation never leaks down into the harness.
fn strip_fences(raw: &str) -> &str {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("```json")
        && let Some(inner) = rest.strip_suffix("```")
    {
        return inner.trim();
    }
    if let Some(rest) = trimmed.strip_prefix("```")
        && let Some(inner) = rest.strip_suffix("```")
    {
        return inner.trim();
    }
    trimmed
}
