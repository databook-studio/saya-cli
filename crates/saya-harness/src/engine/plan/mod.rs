//! Plan proposal and validation: the model proposes a plan; the engine
//! decides whether it may run one.
//!
//! The proposal is the easy half — one JSON-mode provider call whose answer
//! is parsed defensively (see `parse`). The validation is the point: a plan
//! is model-proposed, untrusted input, and it binds only after
//! [`RunPlan::validate`] — the contract's own gate, which re-checks every
//! bound a plan arriving as JSON skipped the constructors for — passes
//! against the run's approved scopes and remaining budget. The engine writes
//! none of those checks a second time and never narrows a plan itself: the
//! plan returned is the model's own, validated.
//!
//! An invalid plan is re-prompted with what was wrong, bounded at
//! [`MAX_PLAN_ATTEMPTS`] proposals the first included; the bound spent, the
//! run stops with the typed last refusal — never unbounded, never a silent
//! acceptance of a plan the model did not propose. A step asking for a scope
//! the run was not approved for is refused with
//! [`PlanRejection::NeedsApproval`]: approval is granted once, by the user,
//! and a new capability never inherits an earlier plan's approval
//! (DESIGN §5.3). The approval surface that can act on the ask is M1-10;
//! here the ask is a typed refusal.

mod contract;
mod parse;
mod prompt;

pub use contract::{MAX_PLAN_ATTEMPTS, PlanError, PlanRejection, PlanRequest};
pub use parse::PlanParseFailure;

use saya_agent::{ChatProvider, ResponseFormat};
use saya_types::{Budgets, Capabilities, RunContractError, RunPlan};

/// Proposes and validates a run's plan through one provider.
pub struct PlanDriver<'a> {
    provider: &'a dyn ChatProvider,
    request: PlanRequest,
}

impl<'a> PlanDriver<'a> {
    /// A driver over `provider`, proposing for `request`.
    pub fn new(provider: &'a dyn ChatProvider, request: PlanRequest) -> Self {
        Self { provider, request }
    }

    /// Proposes a plan against the run's approval. `scopes` is the run's
    /// approved capability set (`RunSpec::scopes`); `remaining` is the
    /// budget remaining as of validation — the full run budget when binding
    /// a fresh plan, whatever is left when re-planning at a step boundary.
    ///
    /// Each attempt is one JSON-mode provider call; a refused plan is
    /// re-prompted with its refusal until the bound is spent.
    pub async fn propose(
        &self,
        scopes: &Capabilities,
        remaining: &Budgets,
    ) -> Result<RunPlan, PlanError> {
        let mut last_refusal = None;
        for _ in 0..MAX_PLAN_ATTEMPTS {
            // JSON mode only: a provider that cannot honour an effort
            // variant drops it rather than erroring, so this call depends on
            // the response shape alone and leaves effort to the endpoint
            // (the learning runner's note).
            let request = prompt::request(&self.request, scopes, remaining, last_refusal.as_ref())
                .with_response_format(ResponseFormat::JsonObject);
            let response = self
                .provider
                .complete(request)
                .await
                .map_err(|source| PlanError::Provider { source })?;
            let plan = match parse::parse_plan(&response.message.content) {
                Ok(plan) => plan,
                Err(kind) => {
                    last_refusal = Some(PlanRejection::Malformed { kind });
                    continue;
                }
            };
            match plan.validate(scopes, remaining) {
                Ok(()) => return Ok(plan),
                Err(source) => last_refusal = Some(refusal_of(&plan, source)),
            }
        }
        Err(PlanError::Exhausted {
            attempts: MAX_PLAN_ATTEMPTS,
            last: last_refusal.expect("at least one proposal precedes exhaustion"),
        })
    }
}

/// Maps the contract's own rejection — which already names the step — onto
/// the driver's typed refusal. The capability rejection is surfaced as its
/// own outcome: it is an approval ask, not an ordinary invalidity.
fn refusal_of(plan: &RunPlan, source: RunContractError) -> PlanRejection {
    match source {
        RunContractError::CapabilityNotApproved(step) => PlanRejection::NeedsApproval { step },
        RunContractError::StepBudgetExceeded(step) => PlanRejection::BudgetTooWide { step },
        RunContractError::EndpointNotBound(step) => PlanRejection::EndpointUnbound {
            step,
            // `validate` refuses only a step that names a role, so the step
            // carries one; the fallback stays typed rather than panicking.
            role: plan.steps[step].endpoint.clone().unwrap_or_default(),
        },
        source => PlanRejection::Invalid { source },
    }
}
