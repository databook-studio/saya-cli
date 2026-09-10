//! The episode brief: what one step's episode is told, and what it may
//! reach.
//!
//! Three derivations, each from the plan and the workspace alone — a run
//! must be reproducible from its spec and config alone (plan G3), so none
//! of them reads the environment:
//!
//! 1. The brief text: the plan state (steps in order, the step being
//!    driven), the step's goal and expected outputs, and the workspace
//!    manifest — names, sizes, digests, never bulk contents.
//! 2. The step's tool definitions: a tool outside the step's capabilities
//!    is **absent** from the list — hidden, not advertised-and-refused.
//!    Candidate-write tools are not capability-scoped, so they pass the
//!    narrowing and are stopped by the pinned limit instead.
//! 3. The step's `AgentLimits`: the step's budget becomes the ceilings, and
//!    learning is pinned off by construction rather than inherited from the
//!    user's memory mode.

use std::cmp::Ordering;
use std::fmt::Write as _;

use saya_agent::{AgentLimits, LocalStateEffect, ToolDefinition};
use saya_types::{Capabilities, RunPlan, StepSpec};

use crate::workspace::Workspace;
use crate::workspace::manifest::{self, ManifestEntry};

use super::{EpisodeRequest, ManifestBounds};

/// Walks the workspace for the brief: names, sizes, digests — never bulk
/// contents. An empty workspace is an empty manifest.
pub(super) fn manifest(
    workspace: &Workspace,
    bounds: &ManifestBounds,
) -> Result<Vec<ManifestEntry>, crate::HarnessError> {
    manifest::build(workspace, bounds.max_files, bounds.max_file_bytes)
}

/// Renders the brief for `plan`'s step `step`: the plan state, the step's
/// goal and expected outputs, and the workspace manifest. Steps before the
/// current one are marked `earlier` — the driver never claims they
/// completed; that is the resume's (a later slice's) knowledge to bring.
pub(super) fn render(plan: &RunPlan, step: usize, manifest: &[ManifestEntry]) -> String {
    let mut brief = format!(
        "You are executing one step of an approved run. The plan has {} steps.\n\nPlan:\n",
        plan.steps.len()
    );
    for (index, item) in plan.steps.iter().enumerate() {
        let marker = match index.cmp(&step) {
            Ordering::Less => "earlier",
            Ordering::Equal => "your step",
            Ordering::Greater => "later",
        };
        let _ = writeln!(brief, "{index}. [{marker}] {}", item.goal);
    }
    let spec = &plan.steps[step];
    let _ = writeln!(brief, "\nYour step's goal: {}", spec.goal);
    if spec.expects.is_empty() {
        brief.push_str("Expected outputs: none declared.\n");
    } else {
        brief.push_str("Expected outputs:\n");
        for hint in &spec.expects {
            match hint.description.as_deref() {
                Some(description) => {
                    let _ = writeln!(brief, "- {}: {description}", hint.name);
                }
                None => {
                    let _ = writeln!(brief, "- {}", hint.name);
                }
            }
        }
    }
    brief.push_str("\nWorkspace manifest (path, bytes, sha256):\n");
    if manifest.is_empty() {
        brief.push_str("(the workspace is empty)\n");
    } else {
        for entry in manifest {
            let _ = writeln!(
                brief,
                "- {} ({} bytes, sha256: {})",
                entry.path, entry.size, entry.digest
            );
        }
    }
    brief
}

/// Narrows the run's tool universe to the step's capabilities. A tool whose
/// local-state effect needs a capability the step lacks is **absent** from
/// the result — the hidden-not-advertised pattern: the model never sees a
/// tool it would only be refused on. Candidate-write tools are not
/// capability-scoped, so they pass and are stopped by the pinned limit.
pub(super) fn definitions(
    universe: &[ToolDefinition],
    capabilities: &Capabilities,
) -> Vec<ToolDefinition> {
    universe
        .iter()
        .filter(|tool| match tool.effect.local_state {
            LocalStateEffect::WriteWorkspace => capabilities.workspace_write,
            _ => true,
        })
        .cloned()
        .collect()
}

/// Turns the step's budget into the loop's limits. The budget's turn and
/// tool-call ceilings are the only ceilings: the environment is never read
/// (plan G3), and a ceiling beyond a `usize` is a ceiling the loop cannot
/// count to, taken as the countable maximum rather than invented smaller.
/// Learning is pinned off by construction (DESIGN §5.8): a run episode is
/// a synthetic conversation, not user intent, so the caller's
/// memory-mode-derived permission is deliberately not consulted — that pin
/// is the whole point of this function.
pub(super) fn limits(request: &EpisodeRequest, spec: &StepSpec) -> AgentLimits {
    let _ = request.memory_allows_candidate_writes;
    let ceiling =
        |asked: Option<u64>| asked.map(|asked| usize::try_from(asked).unwrap_or(usize::MAX));
    let budget = spec.budget.as_ref();
    AgentLimits {
        max_turns: ceiling(budget.and_then(|budget| budget.turns)),
        max_tool_calls: ceiling(budget.and_then(|budget| budget.tool_calls)),
        permit_candidate_writes: false,
        permit_workspace_writes: spec.capabilities.workspace_write,
        context_byte_budget: AgentLimits::default().context_byte_budget,
    }
}
