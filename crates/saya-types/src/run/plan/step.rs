use serde::{Deserialize, Serialize};

use super::outputs::{MAX_OUTPUT_HINTS, OutputHint};
use crate::run::scope::is_name_shaped;
use crate::run::spec::validate_goal;
use crate::run::{RunContractError, budget::Budgets, scope::Capabilities};

/// How many credentials one step may declare for its children. Every
/// set-valued plan surface is bounded so a plan cannot smuggle an unbounded
/// approval view; the endpoint bindings' bound is the scale.
pub const MAX_STEP_CREDENTIALS: usize = 8;

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
    /// The credentials this step declares for its children: run-scoped names
    /// the composition root resolves and hands the step's runner member, so
    /// only what a step declared can ever reach a child's environment.
    /// Empty by default — a child's environment is empty by construction,
    /// and only a declared name can ride it. An interpreter step declares
    /// none and the plan-bind gate refuses the combination: model-authored
    /// code can encode, split, and reverse a declared credential, which
    /// turns redaction's adversary from incidental to deliberate (the
    /// interpreter approval's design §5).
    #[serde(default)]
    pub credentials: Vec<String>,
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
            credentials: Vec::new(),
        })
    }

    /// Declares the credentials this step's children may receive, keeping
    /// hand-built plans honest: every entry is a bounded, name-shaped
    /// credential name, and the count is bounded — the same discipline every
    /// set-valued plan field carries.
    pub fn with_credentials(mut self, credentials: Vec<String>) -> Result<Self, RunContractError> {
        validate_credentials(0, &credentials)?;
        self.credentials = credentials;
        Ok(self)
    }
}

/// Checks one step's declared credentials: bounded, every entry name-shaped.
/// `step` is the plan index for the typed error when validating a bound
/// plan; the constructor validates before a step belongs to one, so `0`
/// never reaches a rendered diagnostic there.
pub(super) fn validate_credentials(
    step: usize,
    credentials: &[String],
) -> Result<(), RunContractError> {
    if credentials.len() > MAX_STEP_CREDENTIALS {
        return Err(RunContractError::TooManyStepCredentials(
            step,
            credentials.len(),
        ));
    }
    if !credentials.iter().all(|name| is_name_shaped(name)) {
        return Err(RunContractError::InvalidStepCredential(step));
    }
    Ok(())
}
