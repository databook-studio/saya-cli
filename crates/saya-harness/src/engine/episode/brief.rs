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
///
/// The write-shaped class is one `LocalStateEffect` variant shared by
/// workspace-write, scratch, runner, interpreter and fetch
/// (`http_download`), so the filter asks the same question the permit
/// mapping does — "did this step approve some write-shaped scope?" — not
/// which tool it is: which tool is the composition root's decision, made
/// from the same capabilities the toolset was built from. This is the second
/// lock behind construction; it must never strip a tool the step approved.
pub(super) fn definitions(
    universe: &[ToolDefinition],
    capabilities: &Capabilities,
) -> Vec<ToolDefinition> {
    universe
        .iter()
        .filter(|tool| match tool.effect.local_state {
            LocalStateEffect::WriteWorkspace => {
                capabilities.workspace_write
                    || capabilities.scratch
                    || capabilities.runner.is_some()
                    || capabilities.interpreter.is_some()
                    || capabilities.fetch.is_some()
            }
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
    let caps = &spec.capabilities;
    AgentLimits {
        max_turns: ceiling(budget.and_then(|budget| budget.turns)),
        max_tool_calls: ceiling(budget.and_then(|budget| budget.tool_calls)),
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

#[cfg(test)]
mod tests {
    use super::*;
    use saya_types::{Destination, FetchScope, RunnerScope};

    fn request() -> EpisodeRequest {
        EpisodeRequest {
            model: "mock-model".into(),
            profile_names: Vec::new(),
            memory_allows_candidate_writes: false,
        }
    }

    fn spec(capabilities: &Capabilities) -> StepSpec {
        StepSpec::new("the step", capabilities.clone(), None, Vec::new(), None).unwrap()
    }

    /// The explicit scope→permit mapping: a step approved any write-shaped
    /// scope — workspace-write, scratch, runner, fetch — carries the write
    /// permit, because `LocalStateEffect` has one write-shaped variant and
    /// the loop gates it on this one permit. What the step can actually
    /// write stays bounded by construction: a scope the step lacks leaves
    /// the tool absent from its definitions, so the union cannot admit a
    /// tool the step never saw.
    #[test]
    fn every_write_shaped_scope_maps_onto_the_write_permit() {
        let mut caps = Capabilities::default();
        assert!(
            !limits(&request(), &spec(&caps)).permit_workspace_writes,
            "no write-shaped scope approved, no permit"
        );

        caps.workspace_write = true;
        assert!(limits(&request(), &spec(&caps)).permit_workspace_writes);

        // Scratch, runner, and fetch were refused at `--allow` parse time
        // until their wiring slices landed — fetch is wired now (S2) — but
        // the mapping is the mapping: approving the scope carries the
        // permit, so wiring a tool needs no further limit change.
        let mut scratch = Capabilities::default();
        scratch.scratch = true;
        let mut runner = Capabilities::default();
        runner.runner = Some(RunnerScope::new(vec!["python3".to_owned()]).expect("shaped"));
        let mut fetch = Capabilities::default();
        fetch.fetch = Some(
            FetchScope::new(vec![
                Destination::new("https", "example.com").expect("shaped"),
            ])
            .expect("shaped"),
        );
        for shape in [scratch, runner, fetch] {
            assert!(
                limits(&request(), &spec(&shape)).permit_workspace_writes,
                "a write-shaped scope must carry the write permit: {shape:?}"
            );
        }
    }

    /// The egress permit's half of the mapping: a step that approved a
    /// plan-gated egress scope — fetch, or runner (a child may carry
    /// `net_allow`) — carries `permit_external_effects`, because the loop's
    /// misconfiguration guard would otherwise deny the fetch tools in both
    /// paths regardless of the scope. A step that approved no egress scope
    /// (workspace-write alone included) does not carry it, and the default
    /// keeps it off, so interactive turns are untouched. The union cannot
    /// admit an external tool the step never saw: the definitions were
    /// built from the same capabilities.
    #[test]
    fn an_egress_scope_maps_onto_the_external_effects_permit() {
        let mut default_caps = Capabilities::default();
        assert!(
            !limits(&request(), &spec(&default_caps)).permit_external_effects,
            "no egress scope approved, no permit"
        );

        // workspace-write alone is a write scope, not an egress scope.
        default_caps.workspace_write = true;
        assert!(!limits(&request(), &spec(&default_caps)).permit_external_effects);

        let mut fetch = Capabilities::default();
        fetch.fetch = Some(
            FetchScope::new(vec![
                Destination::new("https", "example.com").expect("shaped"),
            ])
            .expect("shaped"),
        );
        assert!(
            limits(&request(), &spec(&fetch)).permit_external_effects,
            "an approved fetch scope carries the egress permit"
        );
        assert!(
            limits(&request(), &spec(&fetch)).permit_workspace_writes,
            "fetch carries the write permit too: http_download is its write-shaped member"
        );

        let mut runner = Capabilities::default();
        runner.runner = Some(RunnerScope::new(vec!["python3".to_owned()]).expect("shaped"));
        assert!(
            limits(&request(), &spec(&runner)).permit_external_effects,
            "a runner child may carry net_allow egress, so the mapping includes runner"
        );
    }

    /// The definitions filter asks the same question the permit mapping does:
    /// a write-shaped tool survives for a step that approved any write-shaped
    /// scope. Pinned with the real `scratch_sql` definition, because it is
    /// the first write-shaped tool whose approval does not come from
    /// `workspace_write` — a scratch-only step (the `--allow scratch` shape)
    /// must see it, and a step that approved no write-shaped scope must not.
    #[test]
    fn the_write_shaped_filter_keeps_what_a_write_shaped_scope_approved() {
        use crate::scratch::{SCRATCH_SQL_TOOL, ScratchSql};

        let definition = ScratchSql::definition();
        let names = |universe: &[ToolDefinition]| {
            universe
                .iter()
                .map(|tool| tool.name.clone())
                .collect::<Vec<_>>()
        };

        let mut scratch = Capabilities::default();
        scratch.scratch = true;
        assert_eq!(
            names(&definitions(std::slice::from_ref(&definition), &scratch)),
            vec![SCRATCH_SQL_TOOL],
            "a scratch-only step must see scratch_sql — the filter is the \
             second lock, never a strip of an approved tool"
        );

        assert!(
            definitions(std::slice::from_ref(&definition), &Capabilities::default()).is_empty(),
            "a step that approved no write-shaped scope must not see it"
        );

        let mut workspace_write = Capabilities::default();
        workspace_write.workspace_write = true;
        assert_eq!(
            names(&definitions(&[definition], &workspace_write)),
            vec![SCRATCH_SQL_TOOL],
            "a workspace-write-only step carrying the permit passes the \
             class filter; which tool it sees was the builder's decision"
        );
    }
}
