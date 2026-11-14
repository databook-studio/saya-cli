//! The coding root's plan gate: the one place a plan-first job's write surface
//! is opened.
//!
//! A plan-first job (`saya start --plan`, or `/start --plan` in a
//! session) runs its root under a plan posture: the advertisement is the
//! read-shaped definitions — the same rule the session's own plan posture
//! applies — plus `checklist` and this tool. The write family, `delete`, the
//! shell and the delegation tool are therefore *hidden*, not merely denied, and
//! a call to one is refused as an unknown tool, exactly as the loop's
//! advertisement implies.
//!
//! `submit_plan` is the whole of the gate. The model states the plan, the
//! operator answers that one call, and the answer is the gate: an approval
//! opens the surface for the rest of the run, a refusal leaves it closed and
//! the model revises the plan and submits again. The call ends the turn — the
//! model is told to put nothing after it — and the next request is the one that
//! carries the new advertisement.
//!
//! The gate is an approval, not an authority: opening it grants exactly the
//! ceiling the job already had. The composition, `[tools] allow`, the write
//! scope and every tool's own bounds are untouched by it. A job started without
//! `--plan` composes no gate at all, so its advertisement, its prompt and its
//! request bytes are the ones it always had.
//!
//! The gate's own cell ([`PlanGate::cell`]) is the engine's
//! [`PlanGateCell`], so a surface that reads the gate for its advertisement and
//! the policy that decides the calls cannot disagree about whether the write
//! surface is open.

use std::sync::Arc;

use async_trait::async_trait;
use saya_agent::{
    LocalStateEffect, PlanGateCell, ToolConcurrency, ToolDefinition, ToolEffect, ToolError,
    ToolExecutor,
};
use serde_json::{Value, json};

use crate::agent::tools::consent::{Caller, Operator, Prompt};

/// The one model-tool name this module owns.
pub(crate) const SUBMIT_PLAN: &str = "submit_plan";

/// The longest plan the gate accepts, in characters. The operator's prompt
/// shows the plan in full and refuses text it cannot paint in one prompt, so
/// this is that prompt's own bound: a plan longer than the panel can show is
/// refused before anyone is asked to approve text they cannot read.
pub(crate) const MAX_PLAN_CHARS: usize = 2048;

/// The plan gate for one run: who answers it, and whether they have.
pub(crate) struct PlanGate {
    /// The operator the submission is put to. `None` is a run with nobody to
    /// ask — a shape the start path refuses before a job exists, and which
    /// refuses every submission rather than opening a surface nobody approved.
    operator: Option<Arc<dyn Operator>>,
    /// The caller the operator's prompt names, in this tool's own words.
    caller: Caller,
    /// Whether the operator has opened the write surface for this run — the
    /// engine's own cell, shared with the surface that decides what to
    /// advertise. A coding job's gate opens once and never closes again; a
    /// session's closes when the session leaves `plan` ([`PlanGate::close`]).
    opened: PlanGateCell,
}

/// The plan gate's line for `caller`, in its own words: what the operator reads
/// above the plan. Read by the surfaces that assert the wording, so a test and
/// the prompt cannot state it differently.
pub(crate) fn line(caller: &Caller) -> String {
    caller.line("Approve the submitted plan", "approve the submitted plan")
}

impl PlanGate {
    /// The gate for one run. `caller` names the job the prompt is about.
    pub(crate) fn new(operator: Option<Arc<dyn Operator>>, caller: Caller) -> Self {
        Self {
            operator,
            caller,
            opened: PlanGateCell::default(),
        }
    }

    /// The gate's own cell, for a surface that composes an approval engine
    /// around the same gate: the engine admits a write-shaped call exactly when
    /// this cell is open, which is exactly when the surface advertises it.
    pub(crate) fn cell(&self) -> PlanGateCell {
        self.opened.clone()
    }

    /// Whether the operator has opened the write surface for this run. The one
    /// predicate the advertisement and the dispatch gate both read, so the two
    /// cannot disagree about what the model was offered.
    pub(crate) fn is_open(&self) -> bool {
        self.opened.is_open()
    }

    /// Closes the surface again: the approval that opened it belonged to one
    /// plan, so a session that leaves the posture the plan was written for
    /// asks again rather than carrying the old answer into a new task.
    pub(crate) fn close(&self) {
        self.opened.close();
    }

    /// Whether `name` is the gate's own tool.
    pub(crate) fn owns(name: &str) -> bool {
        name == SUBMIT_PLAN
    }

    /// The `submit_plan` definition. Its effect is read-shaped: the gate writes
    /// this run's posture and nothing else, so the plan posture keeps it
    /// advertised, and the approval engine never routes the call to a decider —
    /// the one question this tool has is the one it asks itself.
    pub(crate) fn definition() -> ToolDefinition {
        ToolDefinition {
            name: SUBMIT_PLAN.into(),
            description: "Submit the plan you propose for this job. The operator answers this \
                          call: approval opens the write surface for the rest of the job, and a \
                          refusal leaves it closed so you revise the plan and submit it again. \
                          End the turn with this call and nothing after it. State what changes, \
                          in which paths, and how you will verify it."
                .into(),
            read_only: true,
            parameters: json!({
                "type": "object",
                "properties": {
                    "plan": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": MAX_PLAN_CHARS,
                        "description": "The plan: what you will change, in which paths, and how you will verify it."
                    }
                },
                "required": ["plan"],
                "additionalProperties": false
            }),
            effect: ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: false,
                local_state: LocalStateEffect::WriteSession,
            },
            concurrency: ToolConcurrency::Serial,
            completion: Some("plan put to the operator".into()),
        }
    }

    /// The plan a call carries, or `None` when the arguments are not exactly
    /// one bounded plan: a one-key object whose `plan` is a non-empty string of
    /// at most [`MAX_PLAN_CHARS`] characters.
    fn plan(arguments: &Value) -> Option<&str> {
        let object = arguments.as_object()?;
        if object.len() != 1 {
            return None;
        }
        let plan = object.get("plan")?.as_str()?;
        let chars = plan.chars().count();
        (chars > 0 && chars <= MAX_PLAN_CHARS).then_some(plan)
    }
}

#[async_trait]
impl ToolExecutor for PlanGate {
    async fn execute(&self, name: &str, arguments: Value) -> Result<Value, ToolError> {
        if !Self::owns(name) {
            return Err(ToolError::UnsupportedTool);
        }
        if Self::plan(&arguments).is_none() {
            return Err(ToolError::UnsupportedProperty);
        }
        // A second submission on an open gate is not a second question: the
        // surface is already open, and re-asking would put the same plan to the
        // operator again.
        if self.is_open() {
            return Ok(json!({
                "approved": true,
                "note": "the operator already approved a plan for this job: the write surface is open"
            }));
        }
        let Some(operator) = &self.operator else {
            return Ok(json!({
                "approved": false,
                "note": "no operator is attached to this job, so no plan can be approved; the write surface stays closed"
            }));
        };
        let prompt = Prompt::new(Arc::clone(operator), line(&self.caller));
        if prompt.ask(&arguments).await {
            self.opened.open();
            Ok(json!({
                "approved": true,
                "note": "the operator approved this plan: the write surface is open for the rest of this job"
            }))
        } else {
            Ok(json!({
                "approved": false,
                "note": "the operator refused this plan: the write surface stays closed; revise the plan and submit it again"
            }))
        }
    }
}
