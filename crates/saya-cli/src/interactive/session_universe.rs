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
//! - **Session state is not a child root.** `fs_roots` is the workspace
//!   root exactly — the session state directory (`sessions/<id>/`, holding
//!   the scratch DuckDB and the lock) is outside it, so a child cannot
//!   corrupt its own session's state, and the runner's outcome records land
//!   there instead of in the user's project.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use saya_agent::{ApprovalPolicy, CancellationToken, ToolDefinition, ToolExecutor};
use saya_harness::fetch::{
    DownloadBudget, DownloadLimits, FetchLimits, FetchPolicy, FetchTools, ReqwestTransport,
};
use saya_harness::runner::{RunProgram, SharedCredentialSource, StaticCredentialSource};
use saya_harness::scratch::ScratchSql;

use super::session_definitions;
use super::session_runner::{SessionRunner, compose_runner};
use super::session_workspace::SessionWorkspace;
use crate::agent::tools::{DatabaseTools, RunTools};

/// One interactive session's tool universe.
pub(crate) struct SessionUniverse {
    workspace: Option<SessionWorkspace>,
    scratch: Option<Arc<ScratchSql>>,
    fetch: Option<Arc<FetchTools>>,
    runner: Option<SessionRunner>,
    /// A startup fact the user must see: a pinned root that no longer
    /// exists. Reported, never silent.
    pub(crate) notice: Option<String>,
}

impl SessionUniverse {
    /// A universe with nothing in it — the struct-literal test support for
    /// the TUI's idle apps, which never dispatch a tool. Production composes
    /// through [`SessionUniverse::compose`] only.
    #[cfg(test)]
    pub(crate) fn empty() -> Self {
        Self {
            workspace: None,
            scratch: None,
            fetch: None,
            runner: None,
            notice: None,
        }
    }

    /// Composes the universe once per session process: the workspace binding
    /// (explicit `--workspace`, or the recorded pin on a resume, or the git
    /// worktree top on a fresh start), the scratch database over the
    /// session's state directory, the session fetch wiring, and the runner
    /// where the host proves.
    pub(crate) fn compose(
        runtime: &crate::config::runtime::RuntimeConfig,
        explicit: Option<&Path>,
        pinned_root: Option<&str>,
        walk_when_unpinned: bool,
        cwd: &Path,
        state_dir: &Path,
    ) -> Result<Self, String> {
        let (workspace, notice) = bind_workspace(explicit, pinned_root, walk_when_unpinned, cwd)?;
        // Scratch: one DuckDB file per session state directory, the pinned
        // scratch semantics (external access off, 0600) either way. It is
        // engine state — a file tool cannot reach it and neither can a child.
        let scratch = ScratchSql::open(state_dir).map_err(|error| {
            format!("the session scratch database could not be opened: {error}")
        })?;
        // Fetch: the session-wide policy — HTTPS only, refused ranges still
        // refused, every host consented per call by the approval engine.
        let transport = ReqwestTransport::new()
            .map_err(|error| format!("the fetch transport could not be built: {error}"))?;
        let fetch = Arc::new(FetchTools::new(
            FetchPolicy::session(),
            Arc::new(transport),
            FetchLimits::for_tool_lane(),
            DownloadLimits::default(),
            workspace.as_ref().map(|bound| Arc::clone(&bound.workspace)),
            DownloadBudget::default(),
        ));
        let runner = match workspace.as_ref() {
            Some(bound) => compose_runner(runtime, &bound.root, state_dir)?,
            None => None,
        };
        Ok(Self {
            workspace,
            scratch: Some(Arc::new(scratch)),
            fetch: Some(fetch),
            runner,
            notice,
        })
    }

    /// The canonical workspace root, when one binds — the status header's
    /// and the session record's fact.
    pub(crate) fn root(&self) -> Option<&Path> {
        self.workspace.as_ref().map(|bound| bound.root.as_path())
    }

    /// The containment seam the file tools read and write through, when a
    /// root binds. `None` leaves `workspace_read` denying with its typed
    /// error — the no-root shape.
    pub(crate) fn workspace(&self) -> Option<Arc<saya_harness::workspace::Workspace>> {
        self.workspace
            .as_ref()
            .map(|bound| Arc::clone(&bound.workspace))
    }

    /// The executor: the shared `RunTools` composite over this session's
    /// database tools and members. The runner member is composed per call
    /// onto the proven spawn, carrying the turn's cancellation.
    pub(crate) fn executor(
        &self,
        database: Arc<DatabaseTools>,
        cancellation: &CancellationToken,
    ) -> Arc<dyn ToolExecutor> {
        let runner = self.runner.as_ref().map(|runner| {
            let resolver: SharedCredentialSource =
                Arc::new(StaticCredentialSource::new(Vec::new()));
            Arc::new(
                RunProgram::for_step(
                    runner.spawn.clone(),
                    Some(runner.scope.clone()),
                    None,
                    runner.timeout,
                    resolver,
                )
                .with_record_dir(runner.record_dir.clone())
                .with_cancellation(cancellation.clone()),
            )
        });
        Arc::new(RunTools::compose(
            database,
            self.scratch.clone(),
            self.fetch.clone(),
            runner,
        ))
    }

    /// The turn's definitions: the database tools' own surface plus this
    /// session's write-shaped members — advertised only where a prompt is
    /// possible (the mode can ask and the surface can prompt), each
    /// ask-gated through the approval engine.
    pub(crate) fn definitions(
        &self,
        mode: ApprovalPolicy,
        can_prompt: bool,
        allow_query_data: bool,
        has_state_store: bool,
        permit_candidate_writes: bool,
    ) -> Vec<ToolDefinition> {
        let asks = matches!(mode, ApprovalPolicy::Ask) && can_prompt;
        // The database surface, always with the write permit off — the
        // session's own workspace_write below is the advertised one, and the
        // run-worded definition must not leak into the session's list.
        let mut defs = DatabaseTools::definitions(
            allow_query_data,
            has_state_store,
            permit_candidate_writes,
            false,
        );
        if !asks {
            // Read-only and never cannot prompt: everything write-shaped
            // stays hidden, not advertised.
            return defs;
        }
        if self.workspace.is_some() {
            defs.push(session_definitions::workspace_write());
        }
        defs.push(session_definitions::scratch_sql());
        if self.fetch.is_some() {
            defs.push(session_definitions::http_fetch());
            if self.workspace.is_some() {
                defs.push(session_definitions::http_download());
            }
        }
        if let Some(runner) = self.runner.as_ref() {
            defs.push(session_definitions::run_program(runner.definition.clone()));
        }
        defs
    }
}

/// Binds the workspace: an explicit `--workspace` first; on a resume, the
/// recorded pin; on a fresh session, the git worktree top. A recorded root
/// that no longer exists binds nothing and says so — fail closed, never
/// re-derive-and-hope.
fn bind_workspace(
    explicit: Option<&Path>,
    pinned_root: Option<&str>,
    walk_when_unpinned: bool,
    cwd: &Path,
) -> Result<(Option<SessionWorkspace>, Option<String>), String> {
    if let Some(dir) = explicit {
        let bound = super::session_workspace::bind(Some(dir), cwd)?
            .expect("an explicit bind returns the root");
        return Ok((Some(bound), None));
    }
    let Some(pin) = pinned_root else {
        let bound = if walk_when_unpinned {
            super::session_workspace::bind(None, cwd)?
        } else {
            // A resumed session whose record predates the workspace: it
            // resumes unbound — exactly its old behaviour — never re-derived
            // from wherever the shell happens to be.
            None
        };
        return Ok((bound, None));
    };
    let recorded = PathBuf::from(pin);
    if !recorded.exists() {
        return Ok((
            None,
            Some(format!(
                "the recorded workspace root {pin} no longer exists: no workspace is bound, \
                 so file reads and writes are unavailable this session"
            )),
        ));
    }
    Ok((super::session_workspace::bind(Some(&recorded), cwd)?, None))
}

#[cfg(test)]
#[path = "session_universe_tests.rs"]
mod tests;
