//! The capture hook: what one successful agent `bounded_sql_query` hands the
//! TUI, and what it refuses.
//!
//! `DatabaseTools` carries an optional hook. When the TUI drives the turn, the
//! hook forwards the typed result the model saw — or a refusal with its reason
//! — onto the turn's stream channel, before the loop's `ToolCompleted` for the
//! same call. The refusal reasons (R3): the model's view was truncated or
//! redacted ([`CaptureRefusalReason::ModelViewTruncated`] /
//! [`CaptureRefusalReason::ModelViewRedacted`] — the capture may only hold a
//! result the model received unchanged), or the result is over the accounted
//! capture budget. Headless paths construct no hook, so nothing there changes.

use std::fmt;
use std::sync::Arc;

use saya_agent::shape_tool_result;
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
/// captured result, or the refusal with its reason — never a partial result.
pub(crate) enum CaptureEvent {
    Captured(AgentCapture),
    Refused {
        sql: String,
        connection: String,
        reason: CaptureRefusalReason,
    },
}

/// Why the hook refused to hold one successful agent query's result — the
/// reason the TUI's snapshot refusal names. The model-view reasons come from
/// [`shape_tool_result`] computed with the loop's own budget: a capture may
/// only hold a result the model received unchanged (R3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CaptureRefusalReason {
    /// The model's message was cut to the conversation byte budget: it saw a
    /// truncated prefix, so the full result is not its evidence.
    ModelViewTruncated,
    /// Redaction replaced secret-shaped material before the message reached
    /// the model: it never saw those values.
    ModelViewRedacted,
    /// The result is over the accounted capture budget.
    OverBudget,
}

impl fmt::Debug for CaptureEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CaptureEvent::Captured(capture) => f.debug_tuple("Captured").field(capture).finish(),
            CaptureEvent::Refused {
                sql,
                connection,
                reason,
            } => f
                .debug_struct("Refused")
                .field("sql", sql)
                .field("connection", connection)
                .field("reason", reason)
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
            self.context_byte_budget,
        ));
    }
}

/// The event one successful query produces: captured when the model received
/// the result unchanged and it fits `capture_budget` accounted bytes,
/// refused with its reason otherwise — whole, never a partial.
///
/// The model-view gate (R3) comes first: `shape_tool_result` — the loop's
/// own shaping, at `context_byte_budget`, the SAME budget the loop passes to
/// `tool_message` for this turn — decides whether the model received the
/// result at all. A truncated or redacted model view refuses the capture
/// with that reason: the typed result would be evidence of data the model
/// never saw. A lossless model view is then still bounded by the accounted
/// capture budget, refused as over budget whole.
pub(crate) fn capture_event(
    sql: &str,
    connection: &str,
    entry: &ConnectionEntry,
    executed: &ExecutedQuery,
    capture_budget: usize,
    context_byte_budget: usize,
) -> CaptureEvent {
    let refused = |reason| CaptureEvent::Refused {
        sql: sql.to_owned(),
        connection: connection.to_owned(),
        reason,
    };
    let shaped = shape_tool_result(&executed.value, context_byte_budget);
    if shaped.truncated {
        return refused(CaptureRefusalReason::ModelViewTruncated);
    }
    if shaped.redactions > 0 {
        return refused(CaptureRefusalReason::ModelViewRedacted);
    }
    if accounted_bytes_within(&executed.result, capture_budget).is_none() {
        return refused(CaptureRefusalReason::OverBudget);
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
