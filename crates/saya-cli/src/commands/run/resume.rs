//! `saya run resume <id>` rebuilds a run from its journal and continues it.
//! The CLI restores collaborators from the persisted spec and current config.

use super::mode::RunApproval;
use super::{parse_run_id, runs_dir};
mod continuation;
mod uncertainty;

use crate::config::runtime::RuntimeConfig;
use crate::render::RenderFormat;
use saya_agent::{ApprovalPolicy, CancellationToken};
use saya_store::{RunStore, SqliteStateStore};

pub(super) async fn resume(
    raw_id: &str,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    approval: ApprovalPolicy,
    state: &SqliteStateStore,
    retry_incomplete: bool,
) -> Result<i32, Box<dyn std::error::Error>> {
    let run_id = parse_run_id(raw_id)?;
    // The entry guard, shared with every other entry point into a run
    // (`mode.rs`): a run's approval is its `--allow` scopes; bypass is a
    // session mode. A resumed run would otherwise compose its frozen policy
    // in bypass mode and auto-allow every ask-shaped call on it — refused
    // here, before anything is read or claimed.
    let approval = RunApproval::admit(approval)?;
    let dir = runs_dir().join(run_id.as_str());
    let record = RunStore::get_run(state, &run_id)
        .await
        .map_err(|error| format!("run state store refused the lookup: {error}"))?;
    if record.is_none() {
        return crate::commands::output::failure_message(
            2,
            format!("no run with id {run_id}"),
            format,
        );
    }
    let spec = match super::files::load_spec(&dir) {
        Ok(spec) => spec,
        Err(message) => return crate::commands::output::failure_message(2, message, format),
    };
    let plan = match super::files::load_plan(&dir) {
        Ok(plan) => plan,
        Err(message) => return crate::commands::output::failure_message(2, message, format),
    };
    // Runs are headless on resume exactly as fresh: the decider never prompts.
    let cancellation = CancellationToken::default();
    let inputs = continuation::ResumeInputs {
        spec,
        plan,
        dir,
        state,
    };
    let work = continuation::continue_run(
        runtime,
        inputs,
        format,
        approval,
        cancellation.clone(),
        retry_incomplete,
    );
    tokio::pin!(work);
    match tokio::select! {
        result = &mut work => result,
        _ = tokio::signal::ctrl_c() => {
            cancellation.cancel();
            work.await
        }
    } {
        Ok(exit) => Ok(exit),
        Err(error) => Err(error),
    }
}
