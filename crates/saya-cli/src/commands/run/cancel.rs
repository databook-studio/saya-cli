//! `saya run cancel <id>` — record a run cancelled through the engine's own
//! state machine.
//!
//! A run executes inside the process that started it, so cancelling one with
//! a live holder is refused: the holder owns the run and is cancelled with
//! Ctrl-C. What the CLI can cancel is everything else — a planned, approved,
//! paused, or process-abandoned run — by claiming the directory and recording
//! `Cancelled` through the engine's event sink, so the journal and the store
//! both advance through the machine a fresh run uses.

use super::{parse_run_id, runs_dir};
use crate::config::runtime::RuntimeConfig;
use crate::render::RenderFormat;
use saya_harness::engine::{EngineEventSink, TransitionEvent};
use saya_harness::journal::Journal;
use saya_harness::lock::RunLock;
use saya_store::{RunStore, SqliteStateStore};
use std::sync::Arc;

pub(super) async fn cancel(
    raw_id: &str,
    _runtime: &RuntimeConfig,
    format: RenderFormat,
    state: &SqliteStateStore,
) -> Result<i32, Box<dyn std::error::Error>> {
    let run_id = parse_run_id(raw_id)?;
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
    let lock = match RunLock::acquire(dir.join("lock")) {
        Ok(lock) => lock,
        Err(error) => {
            return crate::commands::output::failure_message(
                3,
                format!(
                    "run {run_id} is held by another engine ({error}); cancel the process \
                     that owns it (Ctrl-C), or cancel it once that process is gone"
                ),
                format,
            );
        }
    };
    let journal = Journal::open(&dir);
    let rebuilt = match journal.rebuild() {
        Ok(rebuilt) => rebuilt,
        Err(error) => {
            return crate::commands::output::failure_message(
                3,
                format!("run journal could not be replayed: {error}"),
                format,
            );
        }
    };
    let Some(position) = super::journal_position(&rebuilt) else {
        return crate::commands::output::failure_message(
            2,
            format!("run {run_id} holds no recorded run"),
            format,
        );
    };
    if position.is_terminal() {
        drop(lock);
        return crate::commands::output::result(
            format!("run {run_id} already finished ({position})"),
            format,
        );
    }
    let sink = EngineEventSink::new(
        run_id.clone(),
        position,
        journal,
        Arc::new(state.clone()) as Arc<dyn saya_store::RunStore>,
        None,
        std::time::Instant::now,
    );
    match sink.record(TransitionEvent::Cancel).await {
        Ok(_) => {
            drop(lock);
            crate::commands::output::result(format!("run {run_id} cancelled"), format)
        }
        Err(error) => crate::commands::output::failure_message(
            3,
            format!("run cancellation could not be recorded: {error}"),
            format,
        ),
    }
}
