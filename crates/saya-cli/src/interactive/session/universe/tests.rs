//! The session universe's composition tests: what an interactive session
//! advertises where a prompt is possible, what refuses at composition, and
//! the containment facts the red tests pin.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use saya_agent::{AgentMode, ApprovalDecider, ApprovalPolicy, CancellationToken, ToolExecutor};
use saya_config::{
    AiProvider, ColorChoice, MemoryMode, OutputFormat, ResolvedAi, ResolvedConfig,
    ResolvedHostCommands, ResolvedJobs, ResolvedMemory, ThemeChoice,
};

use super::{SessionComposition, SessionUniverse};
use crate::mcp::client::McpTools;

// ---------------------------------------------------------------------------
// harness
// ---------------------------------------------------------------------------

include!("tests/harness.rs");
include!("tests/session_startup.rs");

include!("tests/workspace_unavailable.rs");

// ---------------------------------------------------------------------------
// red test 2 — a read-only session sees none of the write-shaped tools
// ---------------------------------------------------------------------------

include!("tests/mode_tool_surfaces.rs");

// ---------------------------------------------------------------------------
// red test 3 — outside a worktree no root binds
// ---------------------------------------------------------------------------

include!("tests/worktree_tool_surfaces.rs");

include!("tests/workspace_paths.rs");

// ---------------------------------------------------------------------------
// the session composes no runner or interpreter door
// ---------------------------------------------------------------------------

include!("tests/runner_removed.rs");
include!("tests/chart_removed.rs");

// ---------------------------------------------------------------------------
// the one `[tools] allow` list narrows this surface like every other
// ---------------------------------------------------------------------------

include!("tests/tools_allow.rs");

include!("tests/sql_posture.rs");

// ---------------------------------------------------------------------------
// the lock, and the session state boundary
// ---------------------------------------------------------------------------

include!("tests/acquisition_lifecycle.rs");

include!("tests/deletion_and_tombstone.rs");

include!("tests/resumed_session_scope.rs");

include!("tests/workspace_notices.rs");

// ---------------------------------------------------------------------------
// Plan hides the write-shaped tools from the model
// ---------------------------------------------------------------------------

/// The write-shaped names Plan must never advertise: the session members,
/// the host lane, and the `shell` — whose external side effect Plan's
/// posture denies.
const PLAN_HIDDEN_TOOLS: [&str; 6] = [
    "write",
    "edit",
    "http_fetch",
    "http_download",
    "run_command",
    "shell",
];

include!("tests/plan_read_only.rs");

include!("tests/policy_advertisement.rs");

include!("tests/plan_enforcement.rs");

// ---------------------------------------------------------------------------
// the session's plan gate opens the write surface by approval
// ---------------------------------------------------------------------------

include!("tests/plan_gate.rs");

// ---------------------------------------------------------------------------
// the grant surface — an explicit workspace-write grant in headless ask
// ---------------------------------------------------------------------------

/// A session policy with `tokens` granted, bound onto the universe exactly
/// the way the session surfaces bind each turn: `session_request::run` and
/// the TUI spawn read the policy they hold and bind its grant store onto
/// the session universe before the turn runs. Returns the policy so a test
/// can resolve through it like the turn's decider does.
fn granted(universe: &SessionUniverse, tokens: &[&str]) -> saya_agent::SessionPolicy {
    let policy = saya_agent::SessionPolicy::new(ApprovalPolicy::Ask);
    for token in tokens {
        policy.grants().grant(token);
    }
    universe.bind_grants(policy.grants());
    policy
}

include!("tests/ask_grant_views.rs");

include!("tests/workspace_write_effects.rs");

// ---------------------------------------------------------------------------
// the grant families — fetch, command
// ---------------------------------------------------------------------------

include!("tests/fetch_grant.rs");

include!("tests/grant_recompute_and_host_refusal.rs");

// ---------------------------------------------------------------------------
// the session's `shell` — the shared command door, mode-aware
// ---------------------------------------------------------------------------

include!("tests/shell_support.rs");
include!("tests/shell_advertise.rs");
include!("tests/shell_approval.rs");
include!("tests/shell_executor.rs");
include!("tests/shell_turn.rs");
include!("tests/shell_background.rs");
include!("tests/shell_host_env.rs");
