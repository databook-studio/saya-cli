//! The plan contract: the ordered [`StepSpec`]s a run's engine validates and
//! binds.
//!
//! A plan is model-proposed, untrusted input. The validating constructors
//! keep hand-built plans honest, but a plan arriving as JSON skips them —
//! which is why [`RunPlan::validate`] re-checks every bound itself and is
//! the gate the engine binds a plan behind.

use serde::{Deserialize, Serialize};

use super::RunContractError;
use super::budget::Budgets;
use super::scope::{Capabilities, is_name_shaped};
use super::spec::validate_goal;

/// A plan is a bounded list: an orchestrating episode proposes it, and no
/// plausible run needs an unbounded number of steps.
pub const MAX_PLAN_STEPS: usize = 64;

/// How many artifacts one step may declare as expected outputs.
pub const MAX_OUTPUT_HINTS: usize = 16;

const MAX_HINT_DESCRIPTION_CHARS: usize = 512;

/// True when `name` is safe to use as a single workspace path component: no
/// separators, no traversal, no control characters, no whitespace.
pub(crate) fn is_artifact_name(name: &str) -> bool {
    is_name_shaped(name)
        && !name.contains('/')
        && !name.contains('\\')
        && name != "."
        && name != ".."
}

/// An artifact a step is expected to produce, named relative to the run
/// workspace. The name is a single path component because it becomes one;
/// the description is bounded free text for the approval view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct OutputHint {
    pub name: String,
    pub description: Option<String>,
}

impl OutputHint {
    pub fn new(
        name: impl Into<String>,
        description: Option<&str>,
    ) -> Result<Self, RunContractError> {
        let name = name.into();
        if !is_artifact_name(&name) {
            return Err(RunContractError::InvalidOutputHint);
        }
        let description = match description.map(str::trim).filter(|d| !d.is_empty()) {
            Some(text) => {
                if text.chars().count() > MAX_HINT_DESCRIPTION_CHARS
                    || text.chars().any(char::is_control)
                {
                    return Err(RunContractError::InvalidOutputHint);
                }
                Some(text.to_string())
            }
            None => None,
        };
        Ok(Self { name, description })
    }
}

/// One unit of work inside a plan: a bounded goal, the capabilities it may
/// use — a subset of the run's approved scopes, checked by
/// [`RunPlan::validate`] — an optional budget within the run's remaining, the
/// artifacts it is expected to produce, and the endpoint role its episodes
/// call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct StepSpec {
    pub goal: String,
    pub capabilities: Capabilities,
    /// `None` inherits the run's budgets as this step's ceilings.
    pub budget: Option<Budgets>,
    pub expects: Vec<OutputHint>,
    /// The endpoint role this step's episodes call, resolved through the
    /// run's endpoint bindings; `None` lets the engine use its default role.
    pub endpoint: Option<String>,
}

impl StepSpec {
    pub fn new(
        goal: impl Into<String>,
        capabilities: Capabilities,
        budget: Option<Budgets>,
        expects: Vec<OutputHint>,
        endpoint: Option<String>,
    ) -> Result<Self, RunContractError> {
        let goal = goal.into();
        validate_goal(&goal)?;
        if let Some(budget) = &budget {
            budget.validate()?;
        }
        if expects.len() > MAX_OUTPUT_HINTS {
            return Err(RunContractError::TooManyOutputHints(expects.len()));
        }
        Ok(Self {
            goal,
            capabilities,
            budget,
            expects,
            endpoint,
        })
    }
}

/// The ordered steps a run will execute, proposed by the orchestrating
/// episode and bound by the engine only after [`RunPlan::validate`] passes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RunPlan {
    pub steps: Vec<StepSpec>,
}

impl RunPlan {
    pub fn new(steps: Vec<StepSpec>) -> Result<Self, RunContractError> {
        if steps.is_empty() {
            return Err(RunContractError::EmptyPlan);
        }
        if steps.len() > MAX_PLAN_STEPS {
            return Err(RunContractError::TooManySteps);
        }
        Ok(Self { steps })
    }

    /// Checks every step against the approved scopes and the remaining
    /// budget. `remaining` is the run's budget as of validation time — the
    /// full run budget when binding a fresh plan, whatever is left when
    /// re-planning at a step boundary. Every bound is re-checked here so a
    /// plan that arrived as JSON is held to the same rules as a constructed
    /// one.
    pub fn validate(
        &self,
        scopes: &Capabilities,
        remaining: &Budgets,
    ) -> Result<(), RunContractError> {
        if self.steps.is_empty() {
            return Err(RunContractError::EmptyPlan);
        }
        if self.steps.len() > MAX_PLAN_STEPS {
            return Err(RunContractError::TooManySteps);
        }
        for (index, step) in self.steps.iter().enumerate() {
            validate_goal(&step.goal)?;
            if let Some(budget) = &step.budget {
                budget.validate()?;
                if !budget.is_within(remaining) {
                    return Err(RunContractError::StepBudgetExceeded(index));
                }
            }
            if !step.capabilities.is_subset_of(scopes) {
                return Err(RunContractError::CapabilityNotApproved(index));
            }
            if let Some(role) = &step.endpoint
                && !scopes.endpoints.contains(role)
            {
                return Err(RunContractError::EndpointNotBound(index));
            }
            if step.expects.len() > MAX_OUTPUT_HINTS {
                return Err(RunContractError::TooManyOutputHints(step.expects.len()));
            }
        }
        Ok(())
    }
}
