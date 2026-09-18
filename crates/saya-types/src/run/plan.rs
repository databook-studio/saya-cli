//! The plan contract: the ordered [`StepSpec`]s a run's engine validates and
//! binds.
//!
//! A plan is model-proposed, untrusted input. The validating constructors
//! keep hand-built plans honest, but a plan arriving as JSON skips them —
//! which is why [`RunPlan::validate`] re-checks every bound itself and is
//! the gate the engine binds a plan behind.
//!
//! A plan's step count carries no ceiling of its own: a plan reaches the
//! parser only as a model response (`saya-harness/src/engine/plan/mod.rs:70`),
//! already bounded by the model's own output cap and the stream cap
//! (`saya-agent/src/protocol/streaming.rs:16`), so no constant second-guesses it.

use serde::{Deserialize, Serialize};

use super::RunContractError;
use super::budget::Budgets;
use super::scope::{Capabilities, is_bare_name, is_name_shaped};
use super::spec::validate_goal;

/// How many artifacts one step may declare as expected outputs.
pub const MAX_OUTPUT_HINTS: usize = 16;

/// How many credentials one step may declare for its children. Every
/// set-valued plan surface is bounded so a plan cannot smuggle an unbounded
/// approval view; the endpoint bindings' bound is the scale.
pub const MAX_STEP_CREDENTIALS: usize = 8;

const MAX_HINT_DESCRIPTION_CHARS: usize = 512;

/// True when `name` is safe to use as a single workspace path component: no
/// separators, no traversal, no control characters, no whitespace. The same
/// bare-name rule a runner program must satisfy, shared with
/// [`is_bare_name`](super::scope::is_bare_name) so the two cannot drift.
pub(crate) fn is_artifact_name(name: &str) -> bool {
    is_bare_name(name)
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

/// One deliverable a step declared, resolved against the run workspace at
/// the step's completion. `artifact` is `None` when the declared deliverable
/// was never produced — recorded, never silently dropped: a run may not look
/// complete while a declared output is absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Deliverable {
    /// The declared name, as the step's hint names it.
    pub name: String,
    /// The artifact as the manifest recorded it — `None` when the step never
    /// produced the deliverable.
    pub artifact: Option<DeliverableArtifact>,
}

impl Deliverable {
    /// A deliverable the step produced: the manifest's size and digest.
    pub fn present(name: impl Into<String>, size: u64, digest: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            artifact: Some(DeliverableArtifact {
                size,
                digest: digest.into(),
            }),
        }
    }

    /// A declared deliverable the step never produced.
    pub fn missing(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            artifact: None,
        }
    }
}

/// The resolved artifact behind a deliverable: its byte size and lowercase
/// hex sha256 — the same manifest discipline the episode brief reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DeliverableArtifact {
    pub size: u64,
    /// Lowercase hex sha256 of the file's bytes, as the manifest read them.
    pub digest: String,
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
fn validate_credentials(step: usize, credentials: &[String]) -> Result<(), RunContractError> {
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
