//! Agent-facing database tool definitions, execution, and display helpers.

#[cfg(test)]
use crate::connection::{ConnectionEntry, ConnectionRegistry};

/// The evidence rule that decides which optional tools a coding-shaped surface
/// advertises: `list`, `delete`, the web tools, and `helpers`.
pub(crate) mod advertisement;
pub(crate) mod child;
/// The one clarification ask: its definition and its fixed result, shared by
/// every surface that advertises it.
pub(crate) mod clarification;
#[cfg(test)]
#[path = "tools/clarification_tool_tests.rs"]
mod clarification_tool_tests;
pub(crate) mod coding;
/// The two consent seams an egressing call passes through — who is asked
/// ([`consent::Operator`]) and who decides ([`consent::OwnedToolApproval`]) —
/// shared by `shell` and `web_fetch`.
pub(crate) mod consent;
mod database_tools;
mod executor;
mod host_argv;
#[cfg(test)]
#[path = "tools/host_argv_tests.rs"]
mod host_argv_tests;
mod knowledge;
/// The one lean, model-facing text and advertised schema for the shared
/// coding tools.
pub(crate) mod lean;
/// The one place a surface applies `[tools] allow` / `--tools`.
mod narrow;
mod run_tools;
/// The one `shell` tool: schema, lane decision, and sandbox composition,
/// shared by every surface that advertises it (the coding root and a run's
/// per-step toolsets).
pub(crate) mod shell_policy;
mod sql_format;
mod tool_calls;
/// The one `web_fetch` tool: the harness's own bounded GET, converted to text
/// and consented per call, shared by the coding root and its helpers.
pub(crate) mod web;
mod workspace;

pub(crate) use database_tools::DatabaseTools;
// The write bound the tool enforces, which the approval prompt states.
pub(crate) use database_tools::WORKSPACE_WRITE_MAX_BYTES;
pub(crate) use narrow::{NarrowedExecutor, admits, narrow, narrowed};
pub(crate) use run_tools::RunTools;
// Whether the workspace holds a notebook, and the removal a surface whose list
// is fixed for its run applies to the notebook edit: the run's per-step
// toolsets decide by them exactly as the coding root does.
pub(crate) use workspace::notebook::{drop_notebook_edit, present as notebook_present};
// Re-exported through `tools` (not the private `database_tools` module) so the
// agent runtime's learning wiring and tests can reach the observation types.
// `ToolObservation` is consumed only by tests; the others by `agent::learning`.
// `OverrideLog` is drained by the runtime to emit `KnowledgeOverridden`.
#[allow(unused_imports)]
pub(crate) use database_tools::{
    DrainedObservations, ObservationLog, ObservationOutcome, OverrideLog, ToolObservation,
};
// The capture hook types: the TUI builds the hook from its stream
// channel; the runtime attaches it to the tools; the messages and their drain
// consume what it produces.
pub(crate) use database_tools::{AgentCapture, CaptureEvent, CaptureHook, CaptureRefusalReason};
#[allow(unused_imports)]
pub(crate) use sql_format::{collapse_whitespace, format_sql};
#[allow(unused_imports)]
pub(crate) use tool_calls::{SqlCall, sql_tool_call, tool_call_detail};

/// Drops a boolean result flag that sits at its documented default, so a
/// model-facing result does not carry `"truncated": false` or
/// `"omitted_exact": true` for the model to decode on every call. Absence
/// reads as the default, which is why every call site documents that default
/// in the tool's own description or parameter schema.
///
/// Only flags whose absence is unambiguous are passed here. A flag that
/// reports a measurement rather than a state — `sandboxed`, `coverage_complete`,
/// a nullable count — is never dropped: there, absence would read as "unknown"
/// rather than as the default.
pub(crate) fn omit_default_flag(value: &mut serde_json::Value, key: &str, default: bool) {
    if value.get(key).and_then(serde_json::Value::as_bool) == Some(default)
        && let Some(object) = value.as_object_mut()
    {
        object.remove(key);
    }
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tools/knowledge/tests/mod.rs"]
mod contract_tools_tests;

#[cfg(test)]
#[path = "tools/format_tests.rs"]
mod format_tests;

#[cfg(test)]
#[path = "tools/observations_tests.rs"]
mod observations_tests;

#[cfg(test)]
#[path = "tools/result_shape_tests.rs"]
mod result_shape_tests;

#[cfg(test)]
#[path = "tools/column_health_tests.rs"]
mod column_health_tests;

#[cfg(test)]
#[path = "tools/join_check_tests.rs"]
mod join_check_tests;

#[cfg(test)]
#[path = "tools/tools_allow_drift_tests.rs"]
mod tools_allow_drift_tests;

#[cfg(test)]
#[path = "tools/workspace_read_tests.rs"]
mod workspace_read_tests;

#[cfg(test)]
#[path = "tools/workspace_search_tests.rs"]
mod workspace_search_tests;

#[cfg(test)]
#[path = "tools/workspace_concurrency_tests.rs"]
mod workspace_concurrency_tests;

#[cfg(test)]
#[path = "tools/workspace_write_tests.rs"]
mod workspace_write_tests;

#[cfg(test)]
#[path = "tools/workspace_edit_tests.rs"]
mod workspace_edit_tests;

#[cfg(test)]
#[path = "tools/workspace_notebook_edit_tests.rs"]
mod workspace_notebook_edit_tests;

#[cfg(test)]
#[path = "tools/workspace_edit_append_tests.rs"]
mod workspace_edit_append_tests;

#[cfg(test)]
#[path = "tools/workspace_edit_continuation_tests.rs"]
mod workspace_edit_continuation_tests;

#[cfg(test)]
#[path = "tools/workspace_edit_docs_tests.rs"]
mod workspace_edit_docs_tests;

#[cfg(test)]
#[path = "tools/workspace/checkpoint_tests.rs"]
mod workspace_checkpoint_tests;

#[cfg(test)]
#[path = "tools/workspace_read_digest_tests.rs"]
mod workspace_read_digest_tests;

#[cfg(test)]
#[path = "tools/lean_text_tests.rs"]
mod lean_text_tests;

#[cfg(test)]
#[path = "tools/advertisement_tests.rs"]
mod advertisement_tests;
