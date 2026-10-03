//! Blocking helpers that execute a command and RETURN a renderable event,
//! instead of printing to stdout (which would corrupt the alternate screen).
//! The TUI renders the returned event into the transcript.

use crate::config::runtime::RuntimeConfig;
use crate::interactive::sql_operation;
use crate::render::TerminalEvent;
use saya_agent::CancellationToken;

/// Runs a raw SQL query without prompting (the TUI owns the screen).
pub(crate) async fn run_sql(
    runtime: &RuntimeConfig,
    profile_name: Option<&str>,
    sql: &str,
    cancellation: &CancellationToken,
) -> TerminalEvent {
    match sql_operation::execute_with_cancellation(runtime, profile_name, sql, false, cancellation)
        .await
    {
        Ok(result) => TerminalEvent::QueryResult { result },
        Err(error) => TerminalEvent::Error {
            message: error.to_string(),
        },
    }
}
