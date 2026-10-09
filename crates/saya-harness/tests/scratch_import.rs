//! Behavioural coverage for the contained CSV-to-scratch import.

use std::{
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};

use saya_agent::{CancellationToken, MAX_TOOL_MESSAGE_BYTES, ToolExecutionContext, ToolExecutor};
use saya_harness::{
    scratch::{ScratchSql, parse_csv, sanitize_headers},
    workspace::Workspace,
};
use serde_json::json;

/// Every await sits under a 60 s guard so a hung import FAILS instead of
/// stalling the suite — the same discipline as the `scratch_sql` battery.
const GUARD: Duration = Duration::from_secs(60);

async fn guarded<F: Future>(future: F) -> F::Output {
    tokio::time::timeout(GUARD, future)
        .await
        .expect("the 60s guard: a hang means the cancel-before-commit contract regressed")
}

/// The loop-assigned context: the loop's tool-message cap and the turn's
/// token (the import applies its own row, byte and wall-clock budgets).
fn context(token: &CancellationToken) -> ToolExecutionContext {
    ToolExecutionContext {
        result_cap: MAX_TOOL_MESSAGE_BYTES,
        cancellation: token.clone(),
    }
}

// ---------------------------------------------------------------------------
// Turn cancellation: a cancelled turn stops a scratch import cooperatively —
// checked before parsing, beside the lock, every row, and immediately before
// the commit; a cancelled import never commits, and the blocking task is
// joined (the await is the settle) before the call returns.

#[path = "scratch_import/csv_shapes.rs"]
mod csv_shapes;

#[path = "scratch_import/import_policy.rs"]
mod import_policy;

#[path = "scratch_import/turn_cancellation.rs"]
mod turn_cancellation;
