//! Run contracts: the specification, plan, scopes, budgets, and event stream
//! a run is declared, approved, and driven with.
//!
//! These are data contracts only — nothing here executes or persists. The
//! engine (in `saya-harness`) enforces what these shapes declare: a plan that
//! asks for capabilities outside the approved scopes is rejected, not
//! coerced, and a budget is a ceiling the engine pauses on, never silently
//! overruns.

pub(crate) mod budget;
pub(crate) mod event;
pub(crate) mod plan;
pub(crate) mod scope;
pub(crate) mod spec;

pub use budget::{Budgets, MAX_BUDGET_ENDPOINTS};
pub use event::{PauseReason, RunEvent, RunFailureCode};
pub use plan::{MAX_OUTPUT_HINTS, MAX_PLAN_STEPS, OutputHint, RunPlan, StepSpec};
pub use scope::{
    Capabilities, Destination, EndpointBindings, FetchScope, MAX_ENDPOINT_BINDINGS,
    MAX_FETCH_DESTINATIONS, MAX_RUNNER_PROGRAMS, RunnerScope, is_name_shaped,
};
pub use spec::{MAX_GOAL_BYTES, RunId, RunSpec};

use thiserror::Error;

/// Why a run contract rejected a value. Variants that name a step carry the
/// step's index in the plan so a diagnostic can point at the offending step.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum RunContractError {
    #[error("run id is not a valid identifier")]
    InvalidRunId,

    #[error("goal must not be empty")]
    EmptyGoal,

    #[error("goal is too long")]
    GoalTooLong,

    #[error("goal contains control characters")]
    GoalControlCharacter,

    #[error("plan must contain at least one step")]
    EmptyPlan,

    #[error("plan has too many steps")]
    TooManySteps,

    #[error("step {0} requests a capability outside the approved scopes")]
    CapabilityNotApproved(usize),

    #[error("step {0} budget exceeds the run's remaining budget")]
    StepBudgetExceeded(usize),

    #[error("step {0} names an endpoint role that the run does not bind")]
    EndpointNotBound(usize),

    #[error("a step declares too many expected outputs ({0})")]
    TooManyOutputHints(usize),

    #[error("output hint is not a valid workspace artifact name")]
    InvalidOutputHint,

    #[error("fetch scope must declare at least one destination")]
    EmptyDestinations,

    #[error("fetch scope declares too many destinations")]
    TooManyDestinations,

    #[error("fetch destination is not a well-shaped scheme and host")]
    InvalidDestination,

    #[error("runner scope must declare at least one program")]
    EmptyPrograms,

    #[error("runner scope declares too many programs")]
    TooManyPrograms,

    #[error("runner program name is not a valid program name")]
    InvalidProgram,

    #[error("endpoint name is not a valid name")]
    InvalidEndpointName,

    #[error("too many endpoint bindings")]
    TooManyEndpointBindings,

    #[error("budget declares token ceilings for too many endpoints")]
    TooManyBudgetEndpoints,
}
