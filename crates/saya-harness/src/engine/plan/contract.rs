//! The plan driver's public contract: its typed errors and the inputs the
//! composition root supplies once per run — the same layout as the episode
//! driver's.
//!
//! These types are constructible by design — they are the caller's side of
//! the driver, not data the model produces.

use saya_agent::ProviderError;
use saya_types::RunContractError;
use thiserror::Error;

use super::PlanParseFailure;

/// How many plans the engine asks the model for, the first included. The
/// proposal loop's bound: spent loudly — the typed last refusal — never
/// unbounded, never a narrowed plan the model did not propose.
pub const MAX_PLAN_ATTEMPTS: usize = 3;

/// Why a plan proposal gave up. Data, not prose: `saya-cli` renders these.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PlanError {
    /// The provider call failed — a transport problem, not a plan problem.
    #[error("plan proposal provider call failed: {source}")]
    Provider {
        #[source]
        source: ProviderError,
    },

    /// The bound is spent: `attempts` proposals, every one refused. `last`
    /// is the typed final refusal — the code the run stops with.
    #[error("plan refused after {attempts} attempt(s); the last refusal: {last}")]
    Exhausted {
        attempts: usize,
        last: PlanRejection,
    },
}

/// One plan refusal, typed by what was wrong. Each variant names the
/// offending step where one exists.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum PlanRejection {
    /// A step asks for a capability outside the run's approved scopes. The
    /// model cannot fix this by re-planning; the user must grant it — the
    /// approval surface that can is M1-10.
    #[error("step {step} asks for a capability outside the run's approved scopes")]
    NeedsApproval { step: usize },

    /// A step's budget exceeds the run's remaining budget on some ceiling —
    /// a step cannot widen the run.
    #[error("step {step} budget exceeds the run's remaining budget")]
    BudgetTooWide { step: usize },

    /// A step names an endpoint role the run has no binding for.
    #[error("step {step} names endpoint role `{role}`, which the run does not bind")]
    EndpointUnbound { step: usize, role: String },

    /// The response is not a plan: prose, truncated JSON, or the wrong shape.
    #[error("the model's response was not a plan: {kind}")]
    Malformed { kind: PlanParseFailure },

    /// The plan violates a plan-contract bound (empty, too many steps, a
    /// goal out of shape, a malformed budget map, ...).
    #[error("the plan violates a plan contract bound: {source}")]
    Invalid {
        #[source]
        source: RunContractError,
    },
}

/// The per-run inputs of a plan proposal: the model to propose through and
/// the run goal the plan decomposes.
pub struct PlanRequest {
    pub model: String,
    pub run_goal: String,
}
