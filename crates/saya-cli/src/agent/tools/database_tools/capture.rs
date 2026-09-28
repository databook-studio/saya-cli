//! The capture hook: what one successful agent `bounded_sql_query` hands the
//! TUI, and what it refuses.
//!
//! `DatabaseTools` carries an optional hook. When the TUI drives the turn, the
//! hook forwards the typed result the model saw — or a budget refusal — onto
//! the turn's stream channel, before the loop's `ToolCompleted` for the same
//! call. Headless paths construct no hook, so nothing there changes.

use std::fmt;
use std::sync::Arc;

use saya_types::{QueryResult, SqlDialect};

use super::DatabaseTools;
use crate::agent::state_tools::ExecutedQuery;
use crate::connection::ConnectionEntry;
use crate::interactive::tui::capture::accounted_bytes_within;

/// The typed result of one successful agent `bounded_sql_query`, exactly as
/// the model saw it (connector-capped at the model row cap), with the facts
/// the TUI needs to pair it with its pending query and label its evidence.
/// No serde derives: this never enters a session file or any persisted
/// record. `Debug` is hand-written — it is what the pairing tests print.
pub(crate) struct AgentCapture {
    pub(crate) sql: String,
    /// The connection that actually ran the query — the registry key the
    /// resolver picked, never the omission when the model named none.
    pub(crate) connection: String,
    /// The connection's opaque profile identity, when it has one.
    pub(crate) profile_identity: Option<String>,
    pub(crate) dialect: SqlDialect,
    pub(crate) result: QueryResult,
    /// The model-facing row cap the connector applied.
    pub(crate) row_cap: usize,
    pub(crate) started_unix_ms: i64,
    pub(crate) finished_unix_ms: i64,
}

impl fmt::Debug for AgentCapture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentCapture")
            .field("sql", &self.sql)
            .field("connection", &self.connection)
            .field("profile_identity", &self.profile_identity)
            .field("dialect", &self.dialect)
            .field("result", &self.result)
            .field("row_cap", &self.row_cap)
            .field("started_unix_ms", &self.started_unix_ms)
            .field("finished_unix_ms", &self.finished_unix_ms)
            .finish()
    }
}

/// What the hook receives for one successful `bounded_sql_query`: the
/// captured result, or the refusal when it is over the accounted budget —
/// never a partial result.
pub(crate) enum CaptureEvent {
    Captured(AgentCapture),
    Refused { sql: String, connection: String },
}

impl fmt::Debug for CaptureEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CaptureEvent::Captured(capture) => f.debug_tuple("Captured").field(capture).finish(),
            CaptureEvent::Refused { sql, connection } => f
                .debug_struct("Refused")
                .field("sql", sql)
                .field("connection", connection)
                .finish(),
        }
    }
}

/// The hook `DatabaseTools` calls after one successful single-connection
/// query. `Fn` (not `FnMut`): the fan-out's concurrent futures share `&self`.
pub(crate) type CaptureHook = Arc<dyn Fn(CaptureEvent) + Send + Sync>;

impl DatabaseTools {
    /// Emits the capture event for one successful `bounded_sql_query` to the
    /// hook: called before the tool's result returns to the loop, so the
    /// message precedes its `ToolCompleted` on the same channel. The
    /// connection recorded is the one that actually ran — the name the model
    /// wrote, or the registry primary when it omitted one; never the
    /// omission.
    pub(super) fn emit_query_capture(
        &self,
        arguments: &serde_json::Value,
        sql: &str,
        entry: &ConnectionEntry,
        executed: &ExecutedQuery,
    ) {
        let Some(hook) = self.capture_hook.as_ref() else {
            return;
        };
        let connection = arguments
            .get("connection")
            .and_then(serde_json::Value::as_str)
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| self.registry.primary_name().to_owned());
        hook(capture_event(
            sql,
            &connection,
            entry,
            executed,
            crate::interactive::tui::capture::CAPTURE_BUDGET_BYTES,
        ));
    }
}

/// The event one successful query produces: captured when the result fits
/// `budget` accounted bytes, refused otherwise — whole, never a partial.
/// `budget` is a parameter so tests can exercise the refusal with a tiny one;
/// production passes the shared capture budget.
pub(crate) fn capture_event(
    sql: &str,
    connection: &str,
    entry: &ConnectionEntry,
    executed: &ExecutedQuery,
    budget: usize,
) -> CaptureEvent {
    if accounted_bytes_within(&executed.result, budget).is_none() {
        return CaptureEvent::Refused {
            sql: sql.to_owned(),
            connection: connection.to_owned(),
        };
    }
    CaptureEvent::Captured(AgentCapture {
        sql: sql.to_owned(),
        connection: connection.to_owned(),
        profile_identity: entry.profile_id.clone(),
        dialect: entry.dialect,
        result: executed.result.clone(),
        row_cap: executed.row_cap,
        started_unix_ms: executed.started_unix_ms,
        finished_unix_ms: executed.finished_unix_ms,
    })
}

#[cfg(test)]
#[path = "capture_tests.rs"]
mod tests;
