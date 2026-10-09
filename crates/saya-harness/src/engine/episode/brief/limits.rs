//! The step's `AgentLimits`: the step's budget becomes the ceilings, and
//! learning is pinned off by construction.

use saya_agent::AgentLimits;

use saya_types::StepSpec;

use crate::engine::episode::EpisodeRequest;

/// Turns the step's budget into the loop's limits. The budget's turn and
/// tool-call ceilings are the only ceilings: the environment is never read
/// (plan G3), and a ceiling beyond a `usize` is a ceiling the loop cannot
/// count to, taken as the countable maximum rather than invented smaller.
/// Learning is pinned off by construction (DESIGN §5.8): a run episode is
/// a synthetic conversation, not user intent, so the caller's
/// memory-mode-derived permission is deliberately not consulted — that pin
/// is the whole point of this function.
pub(crate) fn limits(request: &EpisodeRequest, spec: &StepSpec) -> AgentLimits {
    let _ = request.memory_allows_candidate_writes;
    let ceiling =
        |asked: Option<u64>| asked.map(|asked| usize::try_from(asked).unwrap_or(usize::MAX));
    let budget = spec.budget.as_ref();
    let caps = &spec.capabilities;
    AgentLimits {
        max_turns: ceiling(budget.and_then(|budget| budget.turns)),
        max_tool_calls: ceiling(budget.and_then(|budget| budget.tool_calls)),
        max_continuations: AgentLimits::default().max_continuations,
        permit_candidate_writes: false,
        // The permit means "this step approved some write-shaped scope",
        // not "this step may write the workspace": `LocalStateEffect` has
        // one write-shaped variant and `AgentLimits` one write permit by
        // design, so every write-shaped scope — workspace-write, scratch,
        // runner, interpreter, fetch (`http_download` is fetch's
        // write-shaped member) — maps onto this permit here. This line is
        // the explicit scope→permit mapping; the union cannot smuggle a
        // tool the step never saw, because the step's definitions are built
        // from the same capabilities.
        permit_workspace_writes: caps.workspace_write
            || caps.scratch
            || caps.runner.is_some()
            || caps.interpreter.is_some()
            || caps.fetch.is_some(),
        // The egress permit: the plan-gated external effects (the fetch
        // tools' `external_side_effect` without per-call approval) are
        // approved once by the step's approved scope, so the loop's
        // misconfiguration guard stands down exactly here. The egress union
        // is fetch and runner — not the write-shaped union minus the plain
        // write scope: workspace-write alone carries no egress, and the
        // interpreter family deliberately carries none either, because an
        // interpreter child is wired with an empty `net_allow` (the
        // fail-closed composition; a non-empty one would mean "any host,
        // that port" for model-authored code). A scope the step lacks
        // leaves the tool absent from its definitions, so the union cannot
        // admit an external tool the step never saw: the definitions and
        // this permit are built from the same capabilities.
        permit_external_effects: caps.fetch.is_some() || caps.runner.is_some(),
        context_byte_budget: AgentLimits::default().context_byte_budget,
    }
}
