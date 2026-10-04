//! Blocking helpers that execute a command and RETURN a renderable event,
//! instead of printing to stdout (which would corrupt the alternate screen).
//! The TUI renders the returned event into the transcript.

use crate::config::runtime::RuntimeConfig;
use crate::interactive::sql_execution::SqlExecution;
use crate::interactive::sql_operation;
use crate::render::TerminalEvent;
use saya_agent::CancellationToken;
use std::sync::{Arc, Mutex};

/// Runs a raw SQL query without prompting (the TUI owns the screen).
/// Async for parity with the spawned worker: the body is the executed source
/// envelope (binding + timestamps), written into the task's per-task slot
/// BEFORE the `QueryResult` event is sent, so the completion builds evidence
/// from the EXECUTED source. Errors write nothing and the slot stays empty.
pub(crate) async fn run_sql(
    runtime: &RuntimeConfig,
    profile_name: Option<&str>,
    sql: &str,
    cancellation: &CancellationToken,
    slot: &Arc<Mutex<Option<SqlExecution>>>,
) -> TerminalEvent {
    match sql_operation::execute_with_cancellation_and_source(
        runtime,
        profile_name,
        sql,
        false,
        cancellation,
    )
    .await
    {
        Ok(execution) => {
            let event = TerminalEvent::QueryResult {
                result: execution.result.clone(),
            };
            if let Ok(mut guard) = slot.lock() {
                *guard = Some(execution);
            }
            event
        }
        Err(error) => TerminalEvent::Error {
            message: error.to_string(),
        },
    }
}
