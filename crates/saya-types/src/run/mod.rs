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
pub(crate) mod missing;
pub(crate) mod plan;
pub(crate) mod scope;
pub(crate) mod spec;

pub use budget::{Budgets, MAX_BUDGET_ENDPOINTS};
pub use event::{PauseReason, RunEvent, RunFailureCode};
pub use plan::{
    Deliverable, DeliverableArtifact, MAX_OUTPUT_HINTS, MAX_STEP_CREDENTIALS, OutputHint, RunPlan,
    StepSpec,
};
pub use scope::{
    Capabilities, Destination, EndpointBindings, FetchScope, InterpreterScope,
    MAX_ENDPOINT_BINDINGS, MAX_FETCH_DESTINATIONS, MAX_RUNNER_PROGRAMS, RunnerScope, is_bare_name,
    is_name_shaped, is_refused_runner_program,
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

    #[error("step {0} declares an expected output that is not a valid workspace artifact name")]
    InvalidOutputHintName(usize),

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

    #[error("interpreter scope must declare at least one program")]
    EmptyInterpreterPrograms,

    #[error("interpreter scope declares too many programs")]
    TooManyInterpreterPrograms,

    #[error("interpreter program name is not a valid program name")]
    InvalidInterpreterProgram,

    #[error(
        "interpreter program is not a shell or interpreter the runner refuses — the \
         interpreter family is the refusal list; every other program belongs to the \
         runner family"
    )]
    InterpreterProgramNotRefused,

    #[error("step {0} declares too many credentials ({1})")]
    TooManyStepCredentials(usize, usize),

    #[error("step {0} declares a credential that is not a valid name")]
    InvalidStepCredential(usize),

    #[error("step {0} declares credentials beside an interpreter scope")]
    CredentialsWithInterpreter(usize),

    #[error("endpoint name is not a valid name")]
    InvalidEndpointName,

    #[error("too many endpoint bindings")]
    TooManyEndpointBindings,

    #[error("budget declares token ceilings for too many endpoints")]
    TooManyBudgetEndpoints,
}
