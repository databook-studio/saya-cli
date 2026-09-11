//! Claiming the run: the engine's directory layout, the single-writer lock,
//! the store mirror, and the journal's `RunStarted`.
//!
//! Only called after the refusal checks, so a refused run never created
//! anything. The lock is held for the whole run and released when the
//! composition root returns; a second engine on the same run is refused
//! outright.

use super::exit;
use crate::render::RenderFormat;
use crate::render_run;
use saya_harness::journal::Journal;
use saya_harness::lock::RunLock;
use saya_harness::run_dir::RunDir;
use saya_store::{NewRun, RunStore, SqliteStateStore};
use saya_types::{Budgets, RunEvent, RunId};

/// Claims the run and returns the directory and its lock. The lock is held
/// for the whole run; the caller drops it when the run ends. The claim
/// journal carries the run wire, so the `RunStarted` it appends is the wire's
/// first line — rendered from the journal's own write, never a second copy.
pub(super) async fn claim(
    run_id: &RunId,
    state: &SqliteStateStore,
    spec: &saya_types::RunSpec,
    format: RenderFormat,
) -> Result<(RunDir, RunLock), String> {
    let run_dir = RunDir::create(&super::runs_dir(), run_id)
        .map_err(|error| format!("run directory could not be created: {error}"))?;
    let lock = RunLock::acquire(run_dir.lock_file())
        .map_err(|error| format!("run could not be claimed: {error}"))?;
    RunStore::create_run(
        state,
        NewRun {
            id: run_id.clone(),
            capabilities: store_flags(&spec.scopes),
            budgets: store_budgets(&spec.budgets),
        },
    )
    .await
    .map_err(|error| format!("run state store refused the run: {error}"))?;
    let journal = render_run::wired_journal(Journal::open(run_dir.root()), format);
    journal
        .append(&RunEvent::RunStarted)
        .map_err(|error| format!("run journal refused RunStarted: {error}"))?;
    super::files::persist_spec(run_dir.root(), spec)
        .map_err(|error| format!("run spec could not be persisted: {error}"))?;
    Ok((run_dir, lock))
}

/// Maps a claim failure onto the documented class: a run that cannot be
/// claimed stopped before its first turn (connection/config, 3).
pub(super) fn claim_failure(
    message: String,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    exit::connection_failure(message, format)
}

/// The store's flag mirror of the approved scopes: destination lists, program
/// allowlists, and bindings live in the run spec file, not the store.
fn store_flags(capabilities: &saya_types::Capabilities) -> saya_store::RunCapabilityFlags {
    saya_store::RunCapabilityFlags {
        workspace_write: capabilities.workspace_write,
        fetch: capabilities.fetch.is_some(),
        runner: capabilities.runner.is_some(),
        scratch: capabilities.scratch,
    }
}

/// The declared budgets in the store's integer shape (milliseconds for
/// wall-clock figures).
fn store_budgets(budgets: &Budgets) -> saya_store::RunBudgets {
    saya_store::RunBudgets {
        wall_clock_ms: budgets.wall_clock.map(|ceiling| ceiling.as_millis() as u64),
        tokens_per_endpoint: budgets.tokens_per_endpoint.clone(),
        turns: budgets.turns,
        tool_calls: budgets.tool_calls,
        ..saya_store::RunBudgets::default()
    }
}
