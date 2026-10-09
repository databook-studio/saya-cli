//! One interactive session's tool universe, composed once per session
//! process (UNIFY U1). The run tool members — `workspace_write`,
//! `scratch_sql`, `http_fetch`, `http_download`, `run_program` — ride the
//! same members the run composes: one `Workspace` root, one scratch
//! database, one fetch member, one proven runner spawn cloned per call.
//!
//! Composition is fail-closed at start, exactly like a run's:
//!
//! - **No workspace root, no write-shaped file tools.** Outside a worktree
//!   with no `--workspace`, the root binds nothing; the write-shaped tools
//!   are hidden-not-advertised and `run_program` is absent. Scratch and
//!   fetch need no root and still work.
//! - **`run_program` once per session** — the sandbox, the placement guard,
//!   and the probe are the runner module's (`session_runner`).
//! - **The host-command lane composes once per session** (`session_host`):
//!   wherever a workspace root binds — and never without one: no root, no
//!   lane, structural, because the child's cwd is pinned to the root.
//! - **Session state is not a child root.** `fs_roots` is the workspace
//!   root exactly — the session state directory (`sessions/<id>/`, holding
//!   the scratch DuckDB and the lock) is outside it, so a child cannot
//!   corrupt its own session's state, and the runner's outcome records land
//!   there instead of in the user's project.

use std::path::Path;
use std::sync::Arc;

use saya_agent::{AgentMode, ApprovalPolicy, CancellationToken, ToolDefinition, ToolExecutor};

use crate::agent::learning::LearningBreaker;
use saya_harness::fetch::{
    DownloadBudget, DownloadLimits, FetchLimits, FetchPolicy, FetchTools, ReqwestTransport,
};
use saya_harness::runner::{RunProgram, SharedCredentialSource, StaticCredentialSource};
use saya_harness::scratch::ScratchSql;

use crate::agent::tools::{DatabaseTools, RunTools};
use crate::interactive::session::lifecycle::recovery::RecoveryHandle;
use crate::interactive::session_definitions;
use crate::interactive::session_host::{self, SessionHost};
use crate::interactive::session_runner::{SessionRunner, compose_runner};
use crate::interactive::session_workspace::{SessionWorkspace, bind_from_pins};

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
    pub(crate) state_dir: &'a Path,
    pub(crate) launch: Option<&'a session_host::HostLaunch>,
    pub(crate) path: Option<String>,
    /// A prior universe's scratch tool when this session recomposes. Its
    /// database handle stays alive for TUI snapshots, and a clone rebinds
    /// only the workspace import root.
    pub(crate) scratch: Option<Arc<ScratchSql>>,
    /// The live session's recovery handle, when this composition belongs
    /// to an acquisition. The wrappers (no runtime behind them) and the
    /// test fixtures pass `None`.
    pub(crate) recovery: Option<RecoveryHandle>,
}

/// One interactive session's tool universe.
pub(crate) struct SessionUniverse {
    workspace: Option<SessionWorkspace>,
    scratch: Option<Arc<ScratchSql>>,
    fetch: Option<Arc<FetchTools>>,
    runner: Option<SessionRunner>,
    /// The host-command lane, when a workspace root bound: the executor
    /// config plus the facts the prompts consult. `None` hides the tool
    /// everywhere — hidden, not advertised.
    host: Option<SessionHost>,
    /// Whether a host command ran this session: `run_program`'s prompt gains
    /// the staged-binary integrity line from that moment (§3 rule 6). Shared
    /// by the executor's host member, which sets it after a host child
    /// settles, and the prompt facts, which read it per turn.
    host_ran: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// The session's deny list: bare program names every door refuses
    /// before grant, prompt, and bypass — session-wide, lane-blind. Composed
    /// even when the host lane is off: deny gates the doors every session
    /// already has.
    deny: crate::interactive::session::permissions::deny::SessionDeny,
    /// The session's live task list: seeded once per session process from
    /// the record, written by `tasks_set`, rendered into each turn, synced
    /// back to the record at the turn boundary.
    tasks: crate::interactive::session::tasks::SessionTasks,
    /// The turn's primary connection handle. The approval deciders hold a
    /// clone, and each turn binds the registry `prepare_turn` builds into
    /// it, so a grant suggestion names the database the session is actually
    /// connected to — and a mid-session `/connect` rebinds it for the next
    /// turn, never stale.
    pub(crate) primary: crate::grant_token::TurnPrimary,
    /// The turn's grant view: the live grant store the session surfaces
    /// bind from the policy they hold, once per turn, mirroring
    /// `primary`'s bind. `definitions` reads it for the per-family
    /// availability decision; unbound is the closed shape.
    grants: crate::interactive::session::permissions::grant_surface::GrantView,
    /// The session's post-turn extraction circuit breaker: two consecutive
    /// misses disable learning for the rest of the session. Not persisted —
    /// a new or resumed session always starts enabled.
    learning_breaker: LearningBreaker,
    /// A startup fact the user must see: a pinned root that no longer
    /// exists, or a sandbox probe that did not prove this host. Reported,
    /// never silent.
    pub(crate) notice: Option<String>,
    /// The session's recovery handle, when a live acquisition composed
    /// this universe: the per-turn `DatabaseTools` build attaches the
    /// checkpoint recorder from it. Every recomposition carries the same
    /// handle — one flag and one id for the session process.
    recovery: Option<RecoveryHandle>,
    /// The probe refused this host: `run_program` is absent for the session
    /// and the fact is on the notice. The bypass activation line references
    /// it — under a mode that claims everything runs, the exception is said.
    pub(crate) probe_refused: bool,
}

include!("composition.rs");
include!("workspace_accessors.rs");
include!("executors.rs");
include!("definitions.rs");

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
