//! What the plan-approval surface shows, before anything runs (DESIGN §7):
//! the scopes the run was granted, the plan's steps with the scopes each
//! one asks for, the run's budgets, and the digests of the workspace
//! artifacts as they stand at approval time. The text is deterministic —
//! the body the modal shows and the tests assert on.

use saya_harness::workspace::Workspace;
use saya_harness::workspace::manifest::{ManifestEntry, build as manifest_build};
use saya_types::{Budgets, Capabilities, RunPlan};

/// The approval view: what was granted, what the plan asks for, what it may
/// spend, and what the workspace holds.
pub(crate) struct PlanApprovalView {
    pub(super) goal: String,
    /// The run's approved scopes (`--allow`), as the grammar's words.
    pub(super) approved: Vec<String>,
    pub(super) steps: Vec<StepView>,
    pub(super) budgets: Budgets,
    pub(super) artifacts: Vec<ManifestEntry>,
}

/// One plan step as the approval view recorded it. Carried on the approval
/// request so the TUI's run panel renders the same steps the modal showed.
#[derive(Clone)]
pub(crate) struct StepView {
    pub(crate) goal: String,
    /// The step's requested scopes as the `--allow` grammar's words.
    pub(crate) scopes: Vec<String>,
    pub(crate) endpoint: Option<String>,
}

/// Builds the view from the bound plan, the run's goal and budgets, and the
/// workspace as it stands at approval time (fresh: empty; a re-approval:
/// whatever earlier steps wrote, digested). A manifest walk that refuses is
/// a refusal to show a lying approval view, not an empty one. `carried` are
/// the stated scopes that are per-call grant words (`sql:<connection>`) —
/// appended to the capability words, so the shown (and journaled) approval
/// states every word the run approved; a resume re-derives them from the
/// journal. A run without carried tokens renders today's bytes exactly.
pub(super) fn view_of(
    goal: &str,
    approved: &Capabilities,
    plan: &RunPlan,
    budgets: &Budgets,
    workspace: &Workspace,
    bounds: saya_harness::engine::ManifestBounds,
    carried: &[String],
) -> Result<PlanApprovalView, String> {
    let artifacts =
        manifest_build(workspace, bounds.max_files, bounds.max_file_bytes).map_err(|error| {
            format!("the workspace manifest for approval could not be built: {error}")
        })?;
    let mut approved_words = approved.missing_from(&Capabilities::default());
    approved_words.extend(carried.iter().cloned());
    Ok(PlanApprovalView {
        goal: goal.to_string(),
        approved: approved_words,
        steps: plan
            .steps
            .iter()
            .map(|step| StepView {
                goal: step.goal.clone(),
                // The step's requested scopes as the `--allow` grammar's
                // words: every declared family is missing from an empty
                // approval, so its tokens are exactly what the step asks for.
                scopes: step.capabilities.missing_from(&Capabilities::default()),
                endpoint: step.endpoint.clone(),
            })
            .collect(),
        budgets: budgets.clone(),
        artifacts,
    })
}

/// The interpreter approval's deterministic warning line, for the approved
/// scopes that carry an interpreter token — `None` for every run that did
/// not opt in, so the body a run without the grant shows is today's bytes
/// exactly. The line states what the user is accepting in place of the
/// typed-argv contract's behavioural half — no euphemism, no dilution into
/// a generic "dangerous mode" banner. The sentence is built by the shared
/// helper the session's activation line uses (`approval_text`), so the two
/// surfaces cannot drift; the run's conditional process-fork clause is the
/// fact true for runs, and the headless receipt prints the same line before
/// the first step: what the terminal shows and what the journal records agree.
pub(super) fn interpreter_warning(approved: &[String]) -> Option<String> {
    let mut programs = Vec::new();
    for token in approved {
        if let Some(program) = token.strip_prefix("interpreter:") {
            programs.push(program.to_string());
        }
    }
    if programs.is_empty() {
        return None;
    }
    Some(crate::approval_text::interpreter_warning(
        "run",
        &programs,
        "(where process-fork is granted) any children it spawns run arbitrary argv.",
    ))
}

/// The approval text: deterministic, one line per fact — the body the modal
/// shows and the tests assert on.
pub(super) fn render(view: &PlanApprovalView) -> String {
    let mut text = format!("run approval — goal: {}", view.goal);
    text.push_str(&format!(
        "\napproved scopes: {}",
        if view.approved.is_empty() {
            "(none)".to_string()
        } else {
            view.approved.join(", ")
        }
    ));
    if let Some(warning) = interpreter_warning(&view.approved) {
        text.push('\n');
        text.push_str(&warning);
    }
    text.push_str("\nplan:");
    for (index, step) in view.steps.iter().enumerate() {
        text.push_str(&format!("\n  {}. {}", index + 1, step.goal));
        text.push_str(&format!(
            " [scopes: {}]",
            if step.scopes.is_empty() {
                "(none)".to_string()
            } else {
                step.scopes.join(", ")
            }
        ));
        if let Some(endpoint) = &step.endpoint {
            text.push_str(&format!(" [endpoint: {endpoint}]"));
        }
    }
    text.push_str("\nbudgets:");
    let declared = render_budgets(&view.budgets);
    text.push_str(if declared.is_empty() {
        " (none declared)"
    } else {
        &declared
    });
    text.push_str("\nworkspace artifacts (sha256):");
    if view.artifacts.is_empty() {
        text.push_str(" (none yet)");
    } else {
        for artifact in &view.artifacts {
            text.push_str(&format!(
                "\n  {} ({} bytes, sha256: {})",
                artifact.path, artifact.size, artifact.digest
            ));
        }
    }
    text
}

/// The declared ceilings as `key=value` tokens; undeclared ceilings are
/// absent, never zero.
fn render_budgets(budgets: &Budgets) -> String {
    let mut declared = Vec::new();
    if let Some(wall_clock) = &budgets.wall_clock {
        declared.push(format!("wall-clock={}s", wall_clock.as_secs()));
    }
    if let Some(bytes) = budgets.downloaded_bytes {
        declared.push(format!("downloaded-bytes={bytes}"));
    }
    if let Some(bytes) = budgets.workspace_bytes {
        declared.push(format!("workspace-bytes={bytes}"));
    }
    if let Some(files) = budgets.workspace_files {
        declared.push(format!("workspace-files={files}"));
    }
    if let Some(count) = budgets.process_count {
        declared.push(format!("process-count={count}"));
    }
    if let Some(time) = &budgets.process_time {
        declared.push(format!("process-time={}s", time.as_secs()));
    }
    if let Some(turns) = budgets.turns {
        declared.push(format!("turns={turns}"));
    }
    if let Some(calls) = budgets.tool_calls {
        declared.push(format!("tool-calls={calls}"));
    }
    for (endpoint, tokens) in &budgets.tokens_per_endpoint {
        declared.push(format!("tokens.{endpoint}={tokens}"));
    }
    declared
        .iter()
        .map(|ceiling| format!("\n  {ceiling}"))
        .collect()
}
