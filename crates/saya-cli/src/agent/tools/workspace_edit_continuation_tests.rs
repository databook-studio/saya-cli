//! End-to-end: a run capped mid-file-write resumes with `workspace_edit`'s
//! append variant and the finished file is byte-exact.
//!
//! Why this file lives here, not in `crates/saya-cli/tests/`: the claim needs
//! a real [`Workspace`] wired to the real tool registry, and the `Sandbox`
//! helper in `workspace_edit_tests`/`workspace_edit_append_tests` is exactly
//! that wiring — a temp-dir workspace plus `DatabaseTools::with_registry`.
//! Those unit-test helpers are private to their modules, so this file keeps
//! its own small copy rather than reaching across test modules. The loop
//! itself (`run_agent_with_sink`) is crate-public, so no production change is
//! needed to drive it from here.

use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use saya_agent::{
    AgentEventSink, AgentLimits, AgentRequest, AllowReadOnlyApproval, CancellationToken,
    ChatMessage, ChatProvider, ChatRequest, ToolCall, ToolExecutor,
};

use super::database_tools::DatabaseTools;
use saya_harness::workspace::Workspace;

#[path = "workspace_edit_continuation_tests/capped_run/mod.rs"]
mod capped_run;
