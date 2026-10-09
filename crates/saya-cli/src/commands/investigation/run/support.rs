use super::super::{EXIT_INVESTIGATION_ERROR, run_outcome::RunOutcome};
use crate::commands::{output::failure_message, state};
use crate::render::RenderFormat;
use saya_agent::CancellationToken;
use saya_store::{AuditOperation, AuditStatus, SqliteStateStore};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub(super) fn is_cancelled(cancellation: Option<&CancellationToken>) -> bool {
    cancellation.is_some_and(CancellationToken::is_cancelled)
}

pub(super) fn stopped(format: RenderFormat) -> Result<RunOutcome, Box<dyn std::error::Error>> {
    no_replay(failure_message(
        EXIT_INVESTIGATION_ERROR,
        "the call was cancelled".to_string(),
        format,
    ))
}

/// Wraps a code-only output result — a refusal or failure whose diagnostic
/// is already emitted — as the no-replay outcome.
pub(super) fn no_replay(
    result: Result<i32, Box<dyn std::error::Error>>,
) -> Result<RunOutcome, Box<dyn std::error::Error>> {
    result.map(RunOutcome::plain)
}

/// Every audit on the replay path is a `Query` operation; only status, rows,
/// and truncation differ.
pub(super) async fn audit(
    state_db: &SqliteStateStore,
    identity: &str,
    status: AuditStatus,
    started: &Instant,
    rows: Option<usize>,
    truncated: Option<bool>,
    format: RenderFormat,
) {
    state::audit(
        state_db,
        identity,
        AuditOperation::Query,
        status,
        started.elapsed(),
        rows,
        truncated,
        format,
    )
    .await;
}

/// Wall clock in unix milliseconds (0 before the epoch).
pub(super) fn unix_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}
