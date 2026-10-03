//! Parse a fresh run request and claim its directory before driving.

use super::super::{budget, claim, host::RunRequest, scopes};
use crate::{config::runtime::RuntimeConfig, render::RenderFormat};
use saya_harness::{journal::JournalWire, lock::RunLock, run_dir::RunDir};
use saya_store::SqliteStateStore;
use saya_types::RunSpec;

/// Parses and refuses before anything exists on disk, then claims the run.
/// Usage refusals remain plain errors; a claim failure is already rendered.
pub(super) async fn prepare(
    request: &RunRequest,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    state: &SqliteStateStore,
    wire: Option<JournalWire>,
) -> Result<Prepared, Result<i32, Box<dyn std::error::Error>>> {
    let goal = request
        .goal
        .clone()
        .map(|text| text.trim().to_string())
        .ok_or(Err(
            "run requires a goal: `saya run \"<goal>\" --allow <scopes>`".into(),
        ))?;
    if request.allow.is_empty() {
        return Err(Err(
            "a headless run refuses to start without --allow <scopes>: it cannot \
             prompt for approval mid-run, so its scopes must be declared up front"
                .into(),
        ));
    }
    let approved = scopes::parse(&request.allow, scopes::Surface::Run)
        .map_err(|message| Err(message.into()))?;
    let budgets = budget::parse(&request.budget, &runtime.resolved.jobs.budgets())
        .map_err(|message| Err(message.into()))?;
    let spec = RunSpec::new(
        request.run_id.clone(),
        &goal,
        approved.capabilities.clone(),
        budgets,
    )
    .map_err(|error| Err(format!("run spec refused: {error}").into()))?;
    let (run_dir, lock) = match claim::claim(&request.run_id, state, &spec, format, wire).await {
        Ok(claimed) => claimed,
        Err(message) => return Err(claim::claim_failure(message, format)),
    };
    Ok(Prepared {
        spec,
        allow_tokens: approved.tokens,
        run_dir,
        lock,
    })
}

/// The spec, carried allow words, and single-writer lock left by a claim.
pub(super) struct Prepared {
    pub(super) spec: RunSpec,
    pub(super) allow_tokens: Vec<String>,
    pub(super) run_dir: RunDir,
    pub(super) lock: RunLock,
}
