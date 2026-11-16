//! The per-call approval facts, pinned: one snapshot per tool family, plus
//! the two properties this module exists for —
//!
//! - **agreement**: the terminal prompt and the TUI modal state the same facts
//!   for the same call (both render from `call_facts`, and the modal renders
//!   its `detail` verbatim — pinned in `ui_snapshot_tests`).
//! - **no false facts**: no fact line is produced for a bound the call
//!   cannot demonstrate — a missing composition fact omits the line, never
//!   prints a placeholder.

use crate::agent::tools::DatabaseTools;
use crate::approval_facts::{ApprovalFacts, FetchFacts, call_facts};
use crate::grant_token::grant_token;
use crate::prompt_approval::approval_prompt;
use saya_agent::{ApprovalChoice, ApprovalDecision, ApprovalPolicy, SessionPolicy};
use saya_agent::{LocalStateEffect, SessionGrants, ToolDefinition, ToolEffect};
use saya_harness::fetch::{DownloadBudget, FetchLimits};
use std::path::PathBuf;

fn database_tool(name: &str) -> ToolDefinition {
    DatabaseTools::definitions(true, false, false, false)
        .into_iter()
        .find(|tool| tool.name == name)
        .unwrap_or_else(|| panic!("{name} is defined"))
}

fn session_tool(name: &str) -> ToolDefinition {
    match name {
        // H02a1 owns the advertised definition; the ask's facts key on the
        // name, so the test binds the effect shape inline.
        "edit" => ToolDefinition {
            name: "edit".into(),
            description: String::new(),
            read_only: false,
            parameters: serde_json::json!({"type": "object"}),
            effect: ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: true,
                local_state: LocalStateEffect::WriteWorkspace,
            },
            concurrency: saya_agent::ToolConcurrency::Serial,
            completion: None,
        },
        "write" => crate::interactive::session_definitions::workspace_write(),
        "http_fetch" => crate::interactive::session_definitions::http_fetch(),
        "http_download" => crate::interactive::session_definitions::http_download(),
        other => database_tool(other),
    }
}

/// A fixed workspace root for the sandbox facts: the prompt displays the
/// composed root's path, so a fixed fake keeps the pinned text deterministic
/// (the facts struct states what was composed; it validates nothing).
const WORKSPACE_ROOT: &str = "/home/user/proj";

/// The session-shaped facts bundle: every member composed, the session's own
/// composition facts — the tool-route fetch bounds, the bound workspace
/// root, and the default download wallet.
fn session_facts() -> ApprovalFacts {
    ApprovalFacts {
        row_cap: crate::agent::state_tools::model_row_cap(500),
        sql_timeout_seconds: 60,
        fetch: Some(FetchFacts {
            fetch_body_bytes: FetchLimits::for_tool_lane().max_total_bytes,
            fetch_seconds: FetchLimits::for_tool_lane().time_budget.as_secs(),
            fetch_redirects: FetchLimits::for_tool_lane().max_redirect_hops,
            download: Some(DownloadBudget::new(1024 * 1024 * 1024)),
        }),
        workspace_root: Some(PathBuf::from(WORKSPACE_ROOT)),
        // The host route is uncomposed here: these snapshots pin the
        // contained postures.
        host: None,
        denied_programs: Vec::new(),
        approval_dir: None,
    }
}

fn sql_facts() -> ApprovalFacts {
    ApprovalFacts {
        row_cap: crate::agent::state_tools::model_row_cap(500),
        sql_timeout_seconds: 60,
        ..ApprovalFacts::default()
    }
}

// --- Snapshot per tool family ------------------------------------------------

include!("approval_facts_tests/snapshots.rs");

// --- The plan gate's ask ------------------------------------------------------

include!("approval_facts_tests/plan.rs");

#[path = "approval_facts_tests/workspace_edit.rs"]
mod workspace_edit;

#[path = "approval_facts_tests/fetch_and_download.rs"]
mod fetch_and_download;

#[path = "approval_facts_tests/shell.rs"]
mod shell;

// --- The properties -----------------------------------------------------------

#[path = "approval_facts_tests/property.rs"]
mod property;

#[path = "approval_facts_tests/host_scope.rs"]
mod host_scope;

// --- The session grant says what it covers ----------------------------------

/// The facts behind a host-route `run_command` ask: the same session bundle
/// with the route composed over the fixed root.
fn host_facts() -> ApprovalFacts {
    ApprovalFacts {
        host: Some(crate::approval_facts::HostFacts::for_tests()),
        workspace_root: Some(PathBuf::from(WORKSPACE_ROOT)),
        ..session_facts()
    }
}
