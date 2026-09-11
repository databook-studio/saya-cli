//! `saya run "<goal>"` — the fresh-run entry point.
//!
//! Refuses by construction before anything exists on disk: scopes unstated,
//! no run. `--allow none` is the stated empty approval — a read-only run,
//! since the episode's per-tool-call decider already defaults to read-only
//! (`assembly.rs`) and nothing else is reachable. Then claims the run,
//! proposes and binds a plan, and drives every step blocking. Ctrl-C
//! cancels and exits 130, the pattern `commands/query.rs` uses; every typed
//! outcome lands in the shared exit mapping (`drive.rs`).

use super::budget;
use crate::config::runtime::RuntimeConfig;
use crate::render::RenderFormat;
use saya_agent::{ApprovalPolicy, CancellationToken};
use saya_store::SqliteStateStore;
use saya_types::RunSpec;

/// The fresh-run invocation inputs: the goal prompt, the `--allow` scopes,
/// the budget overrides, and whether the run can interact.
pub(super) struct StartInputs<'a> {
    pub(super) prompt: Option<String>,
    pub(super) allow: &'a [String],
    pub(super) budget_tokens: &'a [String],
    pub(super) can_prompt: bool,
}

pub(super) async fn start(
    inputs: StartInputs<'_>,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    approval: ApprovalPolicy,
    state: &SqliteStateStore,
) -> Result<i32, Box<dyn std::error::Error>> {
    let StartInputs {
        prompt,
        allow,
        budget_tokens,
        can_prompt,
    } = inputs;
    let goal = prompt
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .ok_or("run requires a goal: `saya run \"<goal>\" --allow <scopes>`")?;
    // Refusal by construction, before anything exists on disk: a headless run
    // states its scopes up front or does not start — no run directory, no
    // store row, no prompt. The refusal is about *stating*: `--allow none`
    // states the empty approval and starts a read-only run, so only an
    // absent `--allow` refuses here.
    if allow.is_empty() {
        return Err(
            "a headless run refuses to start without --allow <scopes>: it cannot \
             prompt for approval mid-run, so its scopes must be declared up front"
                .into(),
        );
    }
    let approved = super::scopes::parse(allow)?;
    let budgets = budget::parse(budget_tokens, &runtime.resolved.jobs.budgets())?;
    let run_id = super::new_run_id();
    let spec = RunSpec::new(
        run_id.clone(),
        &goal,
        approved.capabilities.clone(),
        budgets.clone(),
    )
    .map_err(|error| format!("run spec refused: {error}"))?;
    let (run_dir, lock) = match super::claim::claim(&run_id, state, &spec, format).await {
        Ok(claimed) => claimed,
        Err(message) => return super::claim::claim_failure(message, format),
    };
    // Ctrl-C covers everything below: the plan proposal, the approval, and
    // every episode. The plan-approval surface: a run that can interact asks
    // once over the channel (the `tui/agent.rs` pattern, the terminal
    // answering); a headless run's approval is the RunSpec pre-authorization
    // — `--allow` declared the scopes, and the engine refuses any plan
    // outside them.
    let plan_approval = if can_prompt {
        super::ask::terminal()
    } else {
        super::approval::PlanApproval::PreAuthorized
    };
    let cancellation = CancellationToken::default();
    let work = super::drive::drive(super::drive::DriveInputs {
        spec: &spec,
        run_dir,
        state,
        runtime,
        format,
        approval,
        plan_approval: &plan_approval,
        cancellation: cancellation.clone(),
    });
    tokio::pin!(work);
    match tokio::select! {
        result = &mut work => result,
        _ = tokio::signal::ctrl_c() => { cancellation.cancel(); return Ok(130); }
    } {
        Ok(exit) => {
            drop(lock);
            Ok(exit)
        }
        Err(error) => Err(error),
    }
}
