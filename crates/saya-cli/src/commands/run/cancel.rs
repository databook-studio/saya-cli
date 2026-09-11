//! `saya run cancel <id>` — record a run cancelled through the engine's own
//! state machine.
//!
//! A run executes inside the process that started it, so cancelling one with
//! a live holder is refused: the holder owns the run and is cancelled with
//! Ctrl-C. What the CLI can cancel is everything else — a planned, approved,
//! paused, or process-abandoned run — by claiming the directory and recording
//! `Cancelled` through the engine's event sink, so the journal and the store
//! both advance through the machine a fresh run uses.
//!
//! The recording itself is [`record_cancelled`]: the TUI's run panel stops
//! its in-process run through the same function, so the panel and
//! `saya run cancel` write the same durable record, engine path and all.

use super::{parse_run_id, runs_dir};
use crate::config::runtime::RuntimeConfig;
use crate::render::RenderFormat;
use saya_harness::engine::{EngineEventSink, TransitionEvent};
use saya_harness::journal::{Journal, JournalWire};
use saya_harness::lock::RunLock;
use saya_store::{RunStore, SqliteStateStore};
use std::path::Path;
use std::sync::Arc;

/// What a cancellation attempt left behind — the caller renders it.
#[derive(Debug)]
pub(super) enum CancelOutcome {
    /// `Cancelled` was recorded now: journal and store both advanced.
    Recorded,
    /// The run already stands in a terminal state; nothing was written.
    AlreadyFinished(saya_harness::engine::RunState),
    /// The journal holds no run at all.
    NoRecordedRun,
    /// The durable record refused: journal replay or the engine sink failed.
    Refused(String),
}

/// Records `Cancelled` through the engine's own state machine — the same
/// path `saya run cancel` and the TUI panel's stop both take. The caller
/// must hold the run's single-writer lock (the headless command acquires it;
/// the in-process holder owns it already). `wire` is the host's observer, so
/// the panel sees the same event the journal wrote.
pub(super) async fn record_cancelled(
    run_id: &saya_types::RunId,
    dir: &Path,
    state: &SqliteStateStore,
    wire: Option<JournalWire>,
) -> CancelOutcome {
    let journal = match wire {
        Some(wire) => Journal::open(dir).with_wire(wire),
        None => Journal::open(dir),
    };
    let rebuilt = match journal.rebuild() {
        Ok(rebuilt) => rebuilt,
        Err(error) => {
            return CancelOutcome::Refused(format!("run journal could not be replayed: {error}"));
        }
    };
    let Some(position) = super::journal_position(&rebuilt) else {
        return CancelOutcome::NoRecordedRun;
    };
    if position.is_terminal() {
        return CancelOutcome::AlreadyFinished(position);
    }
    let sink = EngineEventSink::new(
        run_id.clone(),
        position,
        journal,
        Arc::new(state.clone()) as Arc<dyn saya_store::RunStore>,
        None,
        None,
        std::time::Instant::now,
    );
    match sink.record(TransitionEvent::Cancel).await {
        Ok(_) => CancelOutcome::Recorded,
        Err(error) => {
            CancelOutcome::Refused(format!("run cancellation could not be recorded: {error}"))
        }
    }
}

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
    match record_cancelled(&run_id, &dir, state, None).await {
        CancelOutcome::Recorded => {
            drop(lock);
            crate::commands::output::result(format!("run {run_id} cancelled"), format)
        }
        CancelOutcome::AlreadyFinished(position) => {
            drop(lock);
            crate::commands::output::result(
                format!("run {run_id} already finished ({position})"),
                format,
            )
        }
        CancelOutcome::NoRecordedRun => {
            drop(lock);
            crate::commands::output::failure_message(
                2,
                format!("run {run_id} holds no recorded run"),
                format,
            )
        }
        CancelOutcome::Refused(message) => {
            drop(lock);
            crate::commands::output::failure_message(3, message, format)
        }
    }
}

#[cfg(test)]
#[path = "cancel_tests.rs"]
mod tests;
