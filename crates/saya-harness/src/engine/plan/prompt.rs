//! The planner prompt: what the model is told about the run, its approval,
//! and the plan shape it must return — and how a refusal is fed back for the
//! bounded re-plan.

use saya_agent::{ChatMessage, ChatRequest};
use saya_types::{Budgets, Capabilities};

use super::{PlanRejection, PlanRequest};

/// The plan shape the model must return: one JSON object, `steps` in
/// execution order. The fields mirror `StepSpec`; `budget` and `endpoint`
/// are optional (`null`).
const SCHEMA: &str = r#"{
  "steps": [
    {
      "goal": "what this step accomplishes",
      "capabilities": {
        "workspace_write": false,
        "fetch": null,
        "runner": null,
        "scratch": false,
        "endpoints": {}
      },
      "budget": null,
      "expects": [{"name": "artifact-name", "description": "optional"}],
      "endpoint": null
    }
  ]
}"#;

const RULES: &str = "### STRICT RULES:
1. Every step's `capabilities` must be a subset of APPROVED CAPABILITIES below: a step may narrow the approval, never widen it. `fetch` and `runner`, when not null, name only destinations and programs the approval names.
2. `budget`, when not null, stays within REMAINING BUDGET below on every ceiling; `null` inherits the run's budgets.
3. `endpoint`, when not null, names a role bound in APPROVED CAPABILITIES' `endpoints`; `null` uses the engine's default role.
4. `expects` names the workspace artifacts the step is expected to produce, one path component each.
5. Return 1 to 64 steps in execution order, as one JSON object - no prose, no markdown fences.";

/// Builds the proposal request for one attempt: the system message carries
/// the planner's role, rules, and schema; the user message carries the run
/// goal, the approved scopes, and the remaining budget — and, on a re-prompt,
/// the previous refusal. The JSON-mode policy is applied by the driver, the
/// caller that knows this is the plan call.
pub(super) fn request(
    plan: &PlanRequest,
    scopes: &Capabilities,
    remaining: &Budgets,
    refusal: Option<&PlanRejection>,
) -> ChatRequest {
    let system = format!(
        "You are SAYA's run planner. Decompose the run's goal into ordered steps and \
         return a single JSON object matching OUTPUT JSON SCHEMA.\n\n{RULES}\n\n\
         ### OUTPUT JSON SCHEMA:\n{SCHEMA}"
    );
    let mut user = format!(
        "### RUN GOAL:\n{}\n\n### APPROVED CAPABILITIES:\n{}\n\n### REMAINING BUDGET (a ceiling left null is unlimited):\n{}",
        plan.run_goal,
        pretty_scopes(scopes),
        pretty_budgets(remaining),
    );
    if let Some(refusal) = refusal {
        user.push_str(&format!(
            "\n\n### REFUSAL OF THE PREVIOUS PLAN:\n{refusal}\n\nPropose a corrected plan as a single JSON object."
        ));
    }
    ChatRequest::new(
        plan.model.as_str(),
        vec![
            ChatMessage::text("system", system),
            ChatMessage::text("user", user),
        ],
    )
}

/// Renders the approved scopes for the prompt. The contract types always
/// serialize; a failure here is a bug, named as one rather than silently
/// blanking the approval view the model plans against.
fn pretty_scopes(scopes: &Capabilities) -> String {
    serde_json::to_string_pretty(scopes).expect("run contract types serialize")
}

/// Renders the remaining budget for the prompt, under the same invariant as
/// `pretty_scopes`.
fn pretty_budgets(remaining: &Budgets) -> String {
    serde_json::to_string_pretty(remaining).expect("run contract types serialize")
}
