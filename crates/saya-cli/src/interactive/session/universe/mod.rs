//! One interactive session's tool universe, composed once per session
//! process. The run tool members — `write`, `http_fetch`,
//! `http_download` — ride the same members the run composes: one
//! `Workspace` root, one fetch member.
//!
//! Composition is fail-closed at start, exactly like a run's:
//!
//! - **No workspace root, no write-shaped file tools.** Outside a worktree
//!   with no `--workspace`, the root binds nothing; the write-shaped tools
//!   are hidden-not-advertised. Fetch needs no root and still works.
//! - **The host-command lane composes once per session** (`session_host`):
//!   wherever a workspace root binds — and never without one: no root, no
//!   lane, structural, because the child's cwd is pinned to the root.
//! - **Session state is not a child root.** The session state directory
//!   (`sessions/<id>/`, holding the lock) is outside the workspace root, so
//!   no session tool can corrupt its own session's state.

use std::path::Path;
use std::sync::Arc;

use saya_harness::fetch::{
    DownloadBudget, DownloadLimits, FetchLimits, FetchPolicy, FetchTools, ReqwestTransport,
};

use crate::agent::tools::coding::plan_gate::PlanGate;
use crate::interactive::session::lifecycle::recovery::RecoveryHandle;
use crate::interactive::session_host::{self, SessionHost};
use crate::interactive::session_workspace::{SessionWorkspace, bind_from_pins};
use crate::mcp::client::McpTools;

/// The full composition's inputs, including the PATH seam: `path: None`
/// is the no-PATH environment. Production fills it from the environment in
/// [`SessionUniverse::compose_with_launch`]; the no-PATH test seam passes
/// `None` directly, so no test mutates process-global state.
pub(crate) struct SessionComposition<'a> {
    pub(crate) runtime: &'a crate::config::runtime::RuntimeConfig,
    pub(crate) explicit: Option<&'a Path>,
    pub(crate) pinned_root: Option<&'a str>,
    pub(crate) walk_when_unpinned: bool,
    pub(crate) cwd: &'a Path,
    pub(crate) launch: Option<&'a session_host::HostLaunch>,
    pub(crate) path: Option<String>,
    /// The live session's recovery handle, when this composition belongs
    /// to an acquisition. The wrappers (no runtime behind them) and the
    /// test fixtures pass `None`.
    pub(crate) recovery: Option<RecoveryHandle>,
    /// The session's MCP servers, already started. Carried forward across a
    /// recomposition rather than composed again: a server starts once per
    /// session process, and a recomposition is not a restart. Each turn
    /// takes this lane's servers under that turn's own consent
    /// ([`crate::mcp::client::McpTools::with_consent`]).
    pub(crate) mcp: Option<Arc<McpTools>>,
}

/// One interactive session's tool universe.
pub(crate) struct SessionUniverse {
    workspace: Option<SessionWorkspace>,
    fetch: Option<Arc<FetchTools>>,
    /// The host-command lane, when a workspace root bound: the executor
    /// config plus the facts the prompts consult. `None` hides the tool
    /// everywhere — hidden, not advertised.
    host: Option<SessionHost>,
    /// The session's deny list: bare program names every door refuses
    /// before grant, prompt, and bypass — session-wide, lane-blind. Composed
    /// even when the host lane is off: deny gates the doors every session
    /// already has.
    deny: crate::interactive::session::permissions::deny::SessionDeny,
    /// The session's live task list: seeded from the record at the start of
    /// each turn, written by `checklist`, rendered into the turn that
    /// follows a change, synced back to the record at the turn boundary.
    tasks: crate::interactive::session::tasks::SessionTasks,
    /// A startup fact the user must see: a pinned root that no longer
    /// exists, a missing PATH on a root-bound session, or an unavailable
    /// Windows host lane. Reported, never silent.
    pub(crate) notice: Option<String>,
    /// The session's recovery handle, when a live acquisition composed
    /// this universe: the per-turn `DatabaseTools` build attaches the
    /// checkpoint recorder from it. Every recomposition carries the same
    /// handle — one flag and one id for the session process.
    recovery: Option<RecoveryHandle>,
    /// The session's MCP servers, started once. `None` where no server is
    /// configured or the mode reaches none, and then no `mcp__` tool is
    /// advertised.
    mcp: Option<Arc<McpTools>>,
    /// The plan gate: the one place the plan posture's write surface opens.
    /// Composed for every session — a mode switch can enter `plan`
    /// mid-session, and the gate the approval opens is the same one the
    /// switch closes.
    plan: Arc<PlanGate>,
    /// The `SessionStart` context, run once at launch. Carried across a
    /// recomposition so the hook runs once per session process, not once per
    /// workspace bind.
    session_hooks: Arc<crate::agent::hooks::SessionHooks>,
}

include!("composition.rs");
include!("workspace_accessors.rs");
include!("plan.rs");

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
