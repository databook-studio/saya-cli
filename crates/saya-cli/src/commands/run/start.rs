//! `saya run "<goal>"` — the fresh-run entry point.
//!
//! Refuses by construction before anything exists on disk: no scopes, no
//! run. Then claims the run, proposes and binds a plan, and drives every
//! step blocking. Ctrl-C cancels and exits 130, the pattern
//! `commands/query.rs` uses; every typed outcome lands in the shared exit
//! mapping (`drive.rs`).

use super::budget;
use crate::config::runtime::RuntimeConfig;
use crate::render::RenderFormat;
use saya_agent::{ApprovalPolicy, CancellationToken};
use saya_store::SqliteStateStore;
use saya_types::RunSpec;

pub(super) async fn start(
    prompt: Option<String>,
    allow: &[String],
    budget_tokens: &[String],
    runtime: &RuntimeConfig,
    format: RenderFormat,
    approval: ApprovalPolicy,
    state: &SqliteStateStore,
) -> Result<i32, Box<dyn std::error::Error>> {
    let goal = prompt
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .ok_or("run requires a goal: `saya run \"<goal>\" --allow <scopes>`")?;
    // Refusal by construction, before anything exists on disk: a headless run
    // states its scopes up front or does not start — no run directory, no
    // store row, no prompt.
    let approved = super::scopes::parse(allow)?;
    if approved.is_empty() {
        return Err(
            "a headless run refuses to start without --allow <scopes>: it cannot \
             prompt for approval mid-run, so its scopes must be declared up front"
                .into(),
        );
    }
    let budgets = budget::parse(budget_tokens, &runtime.resolved.jobs.budgets())?;
    let run_id = super::new_run_id();
    let spec = RunSpec::new(
        run_id.clone(),
        &goal,
        approved.capabilities.clone(),
        budgets.clone(),
    )
    .map_err(|error| format!("run spec refused: {error}"))?;
    let (run_dir, lock) = match super::claim::claim(&run_id, state, &spec).await {
        Ok(claimed) => claimed,
        Err(message) => return super::claim::claim_failure(message, format),
    };
    // Ctrl-C covers everything below: the plan proposal and every episode.
    let cancellation = CancellationToken::default();
    let work = super::drive::drive(
        &spec,
        run_dir,
        state,
        runtime,
        format,
        approval,
        cancellation.clone(),
    );
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
