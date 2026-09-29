//! The shared connector and audit plumbing for the MCP data tools (task
//! Db): the connector factory with the composed runtime's options, and the
//! audit rows a tool execution leaves behind. MCP never prompts for a
//! missing secret — it refuses — so the factory is built with
//! `can_prompt = false`, the same shape the headless CLI runs with.

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use saya_connectors::{ConnectorOptions, DatabaseConnector, build_connector_with_prompt};
use saya_store::{AuditEntry, AuditOperation, AuditStatus, AuditStore, SqliteStateStore};
use saya_types::DatabaseProfile;

use super::context::McpContext;
use super::tools;

/// Builds a connector for one tool call. The composed runtime's
/// `query_timeout_seconds` and `read_only` ride along exactly as
/// `saya query` sets them.
pub(crate) async fn build_connector(
    profile: &DatabaseProfile,
    context: &McpContext,
) -> Result<Box<dyn DatabaseConnector>, String> {
    let resolver = context.runtime.secret_resolver();
    let settings = ConnectorOptions {
        query_timeout_seconds: context.runtime.resolved.query_timeout_seconds,
        read_only: context.runtime.resolved.read_only,
        ..Default::default()
    };
    build_connector_with_prompt(profile, &resolver, settings, false)
        .await
        .map_err(|error| tools::sanitized(&error.to_string()))
}

/// One audit row for an executed tool call, mirroring the `Query` rows
/// `saya query` writes; a store failure is ignored, never surfaced.
pub(crate) async fn audit(
    store: &SqliteStateStore,
    profile_id: &str,
    status: AuditStatus,
    started: &Instant,
    rows: Option<usize>,
    truncated: Option<bool>,
) {
    let mut event = AuditEntry::new(
        profile_id,
        AuditOperation::Query,
        status,
        started.elapsed().as_millis() as u64,
    );
    event.row_count = rows;
    event.truncated = truncated;
    let _ = store.record_audit(event).await;
}

/// Wall clock in unix milliseconds (0 before the epoch).
pub(crate) fn unix_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}
