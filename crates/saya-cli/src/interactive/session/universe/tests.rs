//! The session universe's composition tests: what an interactive session
//! advertises where a prompt is possible, what refuses at composition, and
//! the containment facts the red tests pin.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use saya_agent::{AgentMode, ApprovalPolicy, CancellationToken, ToolExecutor};
use saya_config::{
    AiProvider, ColorChoice, MemoryMode, OutputFormat, ResolvedAi, ResolvedConfig,
    ResolvedFetchJobs, ResolvedHostCommands, ResolvedInterpreterJobs, ResolvedJobs, ResolvedMemory,
    ResolvedRunnerJobs, ThemeChoice,
};

use super::SessionUniverse;

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

// ---------------------------------------------------------------------------
// red test 6 — a program dir inside the session's workspace root refuses
// ---------------------------------------------------------------------------

include!("tests/workspace_paths.rs");

// ---------------------------------------------------------------------------
// the interpreter door — the seam fix (DESIGN §8.1)
// ---------------------------------------------------------------------------

include!("tests/interpreter_grants.rs");

include!("tests/runner_and_sql_posture.rs");

// ---------------------------------------------------------------------------
// the lock, and the per-session scratch boundary
// ---------------------------------------------------------------------------

include!("tests/acquisition_lifecycle.rs");

include!("tests/deletion_and_tombstone.rs");

include!("tests/scratch_recomposition.rs");

include!("tests/resumed_session_scope.rs");

include!("tests/workspace_notices.rs");

// ---------------------------------------------------------------------------
// mode slice 2 — Plan hides the write-shaped tools from the model
// ---------------------------------------------------------------------------

/// The write-shaped names Plan must never advertise: the session members
/// plus the host lane. `run_program` joins the asserted absence only where
/// the runner composes, so the runner-bearing test below names it explicitly
/// while this shared list names the four the plain composition carries.
const PLAN_HIDDEN_TOOLS: [&str; 8] = [
    "workspace_write",
    "workspace_edit",
    "scratch_sql",
    "scratch_import",
    "http_fetch",
    "http_download",
    "run_program",
    "run_command",
];

include!("tests/plan_read_only.rs");

include!("tests/policy_advertisement.rs");

include!("tests/plan_enforcement.rs");

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
// the remaining grant families — scratch, fetch, runner, command (POST-03 G02)
// ---------------------------------------------------------------------------

include!("tests/scratch_and_fetch_grants.rs");

include!("tests/run_family_grants.rs");

include!("tests/grant_recompute_and_host_refusal.rs");
