use serde::{Deserialize, Serialize};

use super::is_artifact_name;
use super::outputs::MAX_OUTPUT_HINTS;
use super::step::{StepSpec, validate_credentials};
use crate::run::spec::validate_goal;
use crate::run::{RunContractError, budget::Budgets, scope::Capabilities};

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
            // A plan arriving as JSON skipped the constructor's credential
            // checks, so the gate re-checks the bound and the shape here.
            validate_credentials(index, &step.credentials)?;
            // The interpreter step's credential refusal (the interpreter
            // approval's design §5): model-authored code can encode, split,
            // and reverse a declared credential, so redaction's adversary
            // against an interpreter child is deliberate, not incidental.
            // Refused at plan-bind, the way an unbound endpoint role is —
            // the model can fix it by re-planning without the credentials.
            if step.capabilities.interpreter.is_some() && !step.credentials.is_empty() {
                return Err(RunContractError::CredentialsWithInterpreter(index));
            }
            if step.expects.len() > MAX_OUTPUT_HINTS {
                return Err(RunContractError::TooManyOutputHints(step.expects.len()));
            }
            // The hint constructors keep names to one workspace-shaped path
            // component; a plan arriving as JSON skipped them, so the gate
            // re-checks every name here — a declared output that would
            // resolve outside the workspace (or anywhere its rules refuse)
            // never binds.
            if !step.expects.iter().all(|hint| is_artifact_name(&hint.name)) {
                return Err(RunContractError::InvalidOutputHintName(index));
            }
        }
        Ok(())
    }
}
