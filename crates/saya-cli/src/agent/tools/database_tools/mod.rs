use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use saya_harness::workspace::Workspace;
use saya_store::{FsSessionStore, SqliteStateStore};

use crate::connection::ConnectionRegistry;
use crate::contracts::RecallReceipt;
use crate::local_metrics::LocalMetrics;

mod capture;
// `pub(super)` so the sibling chart-save tests (registered under
// `agent::tools`) can reach the pure publish step without opening a browser.
pub(super) mod chart_save;
mod chart_tool;
mod column_health;
mod definitions;
mod dispatch;
mod fan_out;
mod join_check;
mod observations;
mod recorder;
mod result_shape;
// A1: request-scoped log of override findings. Mirrors `propose/log.rs`; the
// runtime drains it after the loop to emit one `KnowledgeOverridden` event.
mod override_log;

// `ObservationLog` types the `observations` field; the observation records and
// the drained log are re-exported so the agent runtime's learning wiring
// (`agent::learning`) and the sibling integration test (`observations_tests`)
// can reach them without the private `observations` submodule being public.
pub(crate) use observations::{
    DrainedObservations, ObservationLog, ObservationOutcome, ToolObservation,
};
// `OverrideLog` types the `override_log` field; re-exported so the runtime can
// drain it to emit one `KnowledgeOverridden` event.
pub(crate) use override_log::OverrideLog;
// The capture hook types (C1): the TUI builds the hook from its stream
// channel; the runtime attaches it to the tools; the drain consumes the
// messages it produces.
pub(crate) use capture::{AgentCapture, CaptureEvent, CaptureHook, CaptureRefusalReason};
// The workspace tool bounds type the workspace tools' harness arguments.
// Re-exported for tests only, so the sibling tests build oversized files and
// over-bound directories against the exact bounds rather than copies of them
// that can go stale. Gated rather than `allow(unused_imports)`: the imports
// genuinely are test-only, and saying so is better than silencing the lint
// that noticed.
#[cfg(test)]
pub(crate) use super::workspace::WORKSPACE_LIST_MAX_ENTRIES;
#[cfg(test)]
pub(crate) use super::workspace::WORKSPACE_READ_MAX_BYTES;
#[cfg(test)]
pub(crate) use super::workspace::{WORKSPACE_GLOB_MAX_MATCHES, WORKSPACE_GLOB_MAX_VISITED};
#[cfg(test)]
pub(crate) use super::workspace::{
    WORKSPACE_GREP_MAX_FILE_BYTES, WORKSPACE_GREP_MAX_LINE_BYTES, WORKSPACE_GREP_MAX_MATCHES,
    WORKSPACE_GREP_MAX_VISITED,
};
// `WORKSPACE_WRITE_MAX_BYTES` is not test-gated: the approval prompt states
// the exact per-write bound the tool enforces (`approval_facts`), so the
// prompt's number is this constant, never a copy of it.
#[cfg(test)]
pub(crate) use super::workspace::WORKSPACE_EDIT_MAX_BYTES;
pub(crate) use super::workspace::WORKSPACE_WRITE_MAX_BYTES;
pub(in crate::agent::tools) use super::workspace::{
    FILE_CHANGED_SINCE_OBSERVED, FILE_EXISTS_REQUIRES_DIGEST, MISSING_FILE_NOT_A_REPLACE,
};

/// Agent tools for inspecting and querying configured database connections.
pub(crate) struct DatabaseTools {
    // `pub(super)` so the sibling `knowledge` module (also a child of
    // `agent::tools`) can resolve a connection and read the privacy/store flags
    // without re-deriving provider policy.
    pub(super) registry: ConnectionRegistry,
    pub(super) max_rows: usize,
    pub(super) allow_query_data: bool,
    pub(super) state_db: Option<SqliteStateStore>,
    pub(super) max_concurrent_fan_out_queries: usize,
    pub(super) fan_out_query_timeout: Duration,
    /// Request-scoped observation collector. `None` leaves behaviour identical
    // to before Phase 3b-2; recording is a no-op when absent (spec §4). An
    // `Arc` lets the application operation that creates the log keep a handle to
    // drain it after the turn while the tools hold their own reference.
    pub(super) observations: Option<Arc<ObservationLog>>,
    /// Request-scoped local metrics collector for the turn's question scope.
    /// `None` — every construction that has not called [`Self::with_metrics`]
    /// — leaves behaviour identical: the operation owner records no phase
    /// samples. An `Arc` so the question scope and the tools share one cell.
    pub(super) metrics: Option<Arc<LocalMetrics>>,
    /// Qualified `catalog.schema.object` names of objects whose claims were
    /// supplied to the model this turn via recall.
    pub(super) supplied_objects: Vec<String>,
    /// The turn's recall receipt, shared with the override detector. `None` in
    /// tests that drive the executor without a receipt; an absent receipt means
    /// no detection — never a guess. An `Arc` so the runtime and the
    /// tools share one reference.
    pub(super) recall_receipt: Option<Arc<RecallReceipt>>,
    /// Request-scoped log of override findings, drained by the runtime to emit
    /// one `KnowledgeOverridden` event. `None` in tests; an absent log
    /// means no event, never a side effect. An `Arc` so the runtime can drain
    /// after the tools consume their clone.
    pub(super) override_log: Option<Arc<OverrideLog>>,
    /// The run's contained workspace, the only file I/O a model-facing tool
    /// reaches. Path resolution is delegated entirely to `Workspace::read` —
    /// nothing here resolves a path itself. `None` (every current path, until
    /// the run engine passes one in) leaves `workspace_read` denying with a
    /// typed error: no workspace, no read. An `Arc` so the engine's handle and
    /// the tools share one resolved root.
    pub(super) workspace: Option<Arc<Workspace>>,
    /// The TUI's capture hook (C1): after one successful single-connection
    /// `bounded_sql_query` it receives the typed result the model saw (or the
    /// over-budget refusal), before the tool's result returns to the loop.
    /// `None` — every headless path — leaves behaviour identical: no hook, no
    /// capture, no message.
    pub(super) capture_hook: Option<CaptureHook>,
    /// Default result cap for direct executor calls. Agent-loop calls receive
    /// their exact per-call share through `ToolExecutor::execute_with_result_cap`.
    pub(super) context_byte_budget: usize,
    /// The session's checkpoint recorder for direct file-tool edits. The
    /// session wiring attaches one per turn wherever the session has a
    /// bound workspace and a recovery handle; `None` — the one-shot `ask`
    /// path, `saya run`, and every non-session construction — leaves
    /// behaviour identical: no capture, no cleanup, no `recovery` field
    /// in any result. With a recorder attached but the flag off (the
    /// default) behaviour is identical too.
    pub(super) checkpoints: Option<super::workspace::checkpoint::CheckpointRecorder>,
}

include!("constructors.rs");
include!("attachments.rs");

#[cfg(test)]
mod tests {
    use super::*;
    use saya_agent::ToolExecutor;

    include!("tests/definitions.rs");
    include!("tests/chart_io.rs");
    include!("tests/chart_bounds.rs");
}
