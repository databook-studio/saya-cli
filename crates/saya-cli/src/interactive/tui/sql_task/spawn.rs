//! The worker thread for one dispatched SQL task: a fresh current-thread
//! tokio runtime per command, its worker permit held until the thread
//! function returns.

use super::super::exec;
use super::super::worker_permits::WorkerPermit;
use super::task::SqlTask;
use crate::config::runtime::RuntimeConfig;
use crate::render::TerminalEvent;
use saya_agent::CancellationToken;
use std::sync::Arc;
use std::sync::mpsc::Receiver;

/// Spawns the query on a worker thread with its own tokio runtime and returns
/// a non-blocking receiver for its [`TerminalEvent`]. The permit acquired by
/// admission is moved into the thread: it is released only when this thread
/// function returns — normally, on error, or on panic — so a detached worker
/// keeps its slot and the cap stays honest.
pub(crate) fn spawn(
    permit: WorkerPermit,
    runtime: Arc<RuntimeConfig>,
    task: SqlTask,
    cancellation: CancellationToken,
) -> Receiver<TerminalEvent> {
    let (tx, rx) = std::sync::mpsc::channel();
    let slot = Arc::clone(&task.execution);
    std::thread::spawn(move || {
        let _permit = permit;
        let event = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime_handle) => runtime_handle.block_on(exec::run_sql(
                &runtime,
                task.profile.as_deref(),
                &task.sql,
                &cancellation,
                &slot,
            )),
            Err(error) => TerminalEvent::Error {
                message: error.to_string(),
            },
        };
        let _ = tx.send(event);
    });
    rx
}
