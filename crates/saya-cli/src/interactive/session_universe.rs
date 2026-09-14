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
//!   off unless stated at launch (`--host-commands`, a `--allow command:<x>`
//!   seed, or user-layer `[host_commands] enable`), and never without a
//!   bound workspace root — no root, no lane, even with the flag.
//! - **Session state is not a child root.** `fs_roots` is the workspace
//!   root exactly — the session state directory (`sessions/<id>/`, holding
//!   the scratch DuckDB and the lock) is outside it, so a child cannot
//!   corrupt its own session's state, and the runner's outcome records land
//!   there instead of in the user's project.

use std::path::Path;
use std::sync::Arc;

use saya_agent::{ApprovalPolicy, CancellationToken, ToolDefinition, ToolExecutor};
use saya_harness::fetch::{
    DownloadBudget, DownloadLimits, FetchLimits, FetchPolicy, FetchTools, ReqwestTransport,
};
use saya_harness::runner::{RunProgram, SharedCredentialSource, StaticCredentialSource};
use saya_harness::scratch::ScratchSql;

use super::session_definitions;
use super::session_host::{self, SessionHost};
use super::session_runner::{PROBE_REFUSED_NOTICE, SessionRunner, compose_runner};
use super::session_workspace::{SessionWorkspace, bind_from_pins};
use crate::agent::tools::{DatabaseTools, RunTools};

/// One interactive session's tool universe.
pub(crate) struct SessionUniverse {
    workspace: Option<SessionWorkspace>,
    scratch: Option<Arc<ScratchSql>>,
    fetch: Option<Arc<FetchTools>>,
    runner: Option<SessionRunner>,
    /// The host-command lane, when the launch stated it and a workspace root
    /// bound: the executor config plus the facts the prompts consult. `None`
    /// hides the tool everywhere — hidden, not advertised.
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
    deny: super::session_deny::SessionDeny,
    /// The turn's primary connection handle. The approval deciders hold a
    /// clone, and each turn binds the registry `prepare_turn` builds into
    /// it, so a grant suggestion names the database the session is actually
    /// connected to — and a mid-session `/connect` rebinds it for the next
    /// turn, never stale.
    pub(crate) primary: crate::grant_token::TurnPrimary,
    /// A startup fact the user must see: a pinned root that no longer
    /// exists, or a sandbox probe that did not prove this host. Reported,
    /// never silent.
    pub(crate) notice: Option<String>,
    /// The probe refused this host: `run_program` is absent for the session
    /// and the fact is on the notice. The bypass activation line references
    /// it — under a mode that claims everything runs, the exception is said.
    pub(crate) probe_refused: bool,
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
            host: None,
            host_ran: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            deny: super::session_deny::SessionDeny::default(),
            primary: crate::grant_token::TurnPrimary::default(),
            notice: None,
            probe_refused: false,
        }
    }

    /// Composes the universe once per session process: the workspace binding
    /// (explicit `--workspace`, or the recorded pin on a resume, or the git
    /// worktree top on a fresh start), the scratch database over the
    /// session's state directory, the session fetch wiring, and the runner
    /// where the host proves. The host lane stays unstated here — off by
    /// construction; the launch path composes through `compose_with_launch`.
    pub(crate) fn compose(
        runtime: &crate::config::runtime::RuntimeConfig,
        explicit: Option<&Path>,
        pinned_root: Option<&str>,
        walk_when_unpinned: bool,
        cwd: &Path,
        state_dir: &Path,
    ) -> Result<Self, String> {
        Self::compose_with_launch(
            runtime,
            explicit,
            pinned_root,
            walk_when_unpinned,
            cwd,
            state_dir,
            None,
        )
    }

    /// Composes with the launch's host-command statement: the flag, the
    /// `--allow command:<x>` seeds, and the user-layer `[host_commands]`.
    /// `None` is the unstated lane — off by construction. The lane composes
    /// only when stated **and** a workspace root binds.
    pub(crate) fn compose_with_launch(
        runtime: &crate::config::runtime::RuntimeConfig,
        explicit: Option<&Path>,
        pinned_root: Option<&str>,
        walk_when_unpinned: bool,
        cwd: &Path,
        state_dir: &Path,
        launch: Option<&session_host::HostLaunch>,
    ) -> Result<Self, String> {
        // The deny list, once per session: the launch's `--deny` refusals
        // plus the user-layer `[session_commands] deny`. Launch-only — a
        // mid-session deny over a held grant would leave a journaled token
        // that gates nothing — so deny precedes every grant by construction.
        // `[jobs.runner] allow` ∩ deny is not an error: the allowlist also
        // governs runs, where deny does not ride; in a session deny wins at
        // the door.
        let mut deny_names: Vec<String> =
            launch.map(|launch| launch.deny_list()).unwrap_or_default();
        deny_names.extend(runtime.resolved.session_deny.programs.iter().cloned());
        let deny = super::session_deny::SessionDeny::from_names(deny_names)?;
        let (workspace, notice) = bind_from_pins(explicit, pinned_root, walk_when_unpinned, cwd)?;
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
        let mut runner_composed = None;
        let mut probe_refused = false;
        if let Some(bound) = workspace.as_ref() {
            let composition = compose_runner(runtime, &bound.root, state_dir)?;
            probe_refused = composition.probe_notice.is_some();
            runner_composed = composition.runner;
        }
        // The host lane, once per session: stated at launch (or in the user
        // layer) and a workspace root bound — no root, no lane, even with
        // the flag. The child's PATH is the parent's own. Unstated (`None`)
        // composes no lane without touching the environment.
        let host = match launch {
            Some(launch) => {
                let path_value = std::env::var("PATH")
                    .map_err(|_| "the host-command lane needs PATH".to_owned())?;
                session_host::compose_host(
                    launch,
                    workspace.as_ref().map(|bound| bound.root.as_path()),
                    path_value,
                )?
            }
            None => None,
        };
        let host_notice = match (&host, launch) {
            (Some(_), Some(launch)) => session_host::launch_notice(
                launch,
                workspace.as_ref().map(|bound| bound.root.clone()),
            ),
            _ => None,
        };
        let notice = notice
            .or(probe_refused.then(|| PROBE_REFUSED_NOTICE.to_owned()))
            .or(host_notice);
        Ok(Self {
            workspace,
            scratch: Some(Arc::new(scratch)),
            fetch: Some(fetch),
            runner: runner_composed,
            host,
            host_ran: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            deny,
            primary: crate::grant_token::TurnPrimary::default(),
            notice,
            probe_refused,
        })
    }

    /// The test seam for the no-root pin: composes the unstated lane over
    /// an unbound workspace — which is exactly the no-root-no-lane shape.
    /// The caller passes its own runtime (the test module's
    /// `session_runtime`); this helper only exists so the pin reads as one
    /// call. Unused outside tests.
    #[cfg(test)]
    pub(crate) fn compose_host_for_tests(
        runtime: &crate::config::runtime::RuntimeConfig,
        cwd: &Path,
        state_dir: &Path,
    ) -> Self {
        Self::compose(runtime, None, None, true, cwd, state_dir)
            .expect("the unstated lane composes without a root")
    }

    /// Whether the host lane composed — the test seam the red tests read.
    #[cfg(test)]
    pub(crate) fn host_composed_for_tests(&self) -> Option<()> {
        self.host.as_ref().map(|_| ())
    }

    /// The canonical workspace root, when one binds — the status header's
    /// and the session record's fact.
    pub(crate) fn root(&self) -> Option<&Path> {
        self.workspace.as_ref().map(|bound| bound.root.as_path())
    }

    /// The approval prompts' facts, read off this universe's composed
    /// members and the resolved config. The prompt states what these
    /// enforce and nothing else: a member that was not composed contributes
    /// no fact lines, and every stated number is the enforcement's own.
    /// The deny list rides along: it gates every program-named door even
    /// with the host lane off, so the deciders read it from here too.
    pub(crate) fn approval_facts(
        &self,
        runtime: &crate::config::runtime::RuntimeConfig,
    ) -> crate::approval_facts::ApprovalFacts {
        // The fetch member was composed with exactly these lane bounds
        // (`compose`: `FetchLimits::for_tool_lane()`), and the same
        // constructor here is the same numbers by construction.
        let lane = saya_harness::fetch::FetchLimits::for_tool_lane();
        crate::approval_facts::ApprovalFacts {
            row_cap: crate::agent::state_tools::model_row_cap(runtime.resolved.max_rows),
            sql_timeout_seconds: runtime.resolved.query_timeout_seconds,
            runner: self.runner.as_ref().map(|runner| runner.prompt_facts()),
            fetch: self
                .fetch
                .as_ref()
                .map(|fetch| crate::approval_facts::FetchFacts {
                    fetch_body_bytes: lane.max_total_bytes,
                    fetch_seconds: lane.time_budget.as_secs(),
                    fetch_redirects: lane.max_redirect_hops,
                    download: Some(fetch.download_budget().clone()),
                }),
            scratch: self.scratch.as_ref().map(|_| {
                // The session's scratch runs under the harness's own
                // constants (opened in `compose` without narrowing).
                crate::approval_facts::ScratchFacts {
                    row_cap: saya_harness::scratch::SCRATCH_ROW_CAP,
                    timeout_seconds: saya_harness::scratch::SCRATCH_QUERY_TIMEOUT.as_secs(),
                }
            }),
            workspace_root: self.root().map(|root| root.to_path_buf()),
            host: self.host.as_ref().map(|host| host.facts.clone()),
            host_ran: self.host_ran.load(std::sync::atomic::Ordering::SeqCst),
            denied_programs: self.deny.programs(),
        }
    }

    /// The containment seam the file tools read and write through, when a
    /// root binds. `None` leaves `workspace_read` denying with its typed
    /// error — the no-root shape.
    pub(crate) fn workspace(&self) -> Option<Arc<saya_harness::workspace::Workspace>> {
        self.workspace
            .as_ref()
            .map(|bound| Arc::clone(&bound.workspace))
    }

    /// The deny list this universe carries — the programs every door
    /// refuses. Read by the session loop's start-event journal site.
    pub(crate) fn deny_programs(&self) -> Vec<String> {
        self.deny.programs()
    }

    /// The shared host-ran flag — the executor's host member sets it after a
    /// host call settles. Cloned into `RunTools` once per executor.
    pub(crate) fn host_ran_flag(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        std::sync::Arc::clone(&self.host_ran)
    }

    /// The executor: the shared `RunTools` composite over this session's
    /// database tools and members, carrying the session deny list. The
    /// runner member is composed per call onto the proven spawn, carrying
    /// the turn's cancellation.
    pub(crate) fn executor(
        &self,
        database: Arc<DatabaseTools>,
        cancellation: &CancellationToken,
    ) -> Arc<dyn ToolExecutor> {
        self.executor_with_journal(database, cancellation, None)
    }

    /// The executor with the session journal attached: a deny firing is
    /// journalled there before the refusal is relayed. The session runtime
    /// is the caller; the journal rides the turn, not the universe.
    pub(crate) fn executor_with_journal(
        &self,
        database: Arc<DatabaseTools>,
        cancellation: &CancellationToken,
        journal: Option<std::sync::Arc<saya_store::SessionJournal>>,
    ) -> Arc<dyn ToolExecutor> {
        let runner = self.runner.as_ref().map(|runner| {
            let resolver: SharedCredentialSource =
                Arc::new(StaticCredentialSource::new(Vec::new()));
            Arc::new(
                RunProgram::for_step(
                    runner.spawn.clone(),
                    Some(runner.scope.clone()),
                    runner.interpreters.clone(),
                    runner.timeout,
                    resolver,
                )
                .with_record_dir(runner.record_dir.clone())
                .with_cancellation(cancellation.clone()),
            )
        });
        let mut tools =
            RunTools::compose(database, self.scratch.clone(), self.fetch.clone(), runner)
                .with_session_deny(self.deny.clone());
        if let Some(journal) = journal {
            tools = tools.with_session_journal(journal);
        }
        let tools = match self.host.as_ref() {
            Some(host) => tools
                .with_host(host.config.clone(), cancellation)
                .with_host_ran(self.host_ran_flag()),
            None => tools,
        };
        Arc::new(tools)
    }

    /// The turn's definitions: the database tools' own surface plus this
    /// session's write-shaped members — advertised exactly where the mode's
    /// consent shape makes them usable: `ask` needs a prompt surface to
    /// answer its asks; `bypass` **is** the consent (no ask, no surface —
    /// the piped-REPL demo runs on this); `read-only` and `never` can
    /// neither prompt nor allow. Each write-shaped call is decided through
    /// the approval engine per call.
    pub(crate) fn definitions(
        &self,
        mode: ApprovalPolicy,
        can_prompt: bool,
        allow_query_data: bool,
        has_state_store: bool,
        permit_candidate_writes: bool,
    ) -> Vec<ToolDefinition> {
        let advertises = match mode {
            ApprovalPolicy::Ask => can_prompt,
            ApprovalPolicy::Bypass => true,
            _ => false,
        };
        // The database surface, always with the write permit off — the
        // session's own workspace_write below is the advertised one, and the
        // run-worded definition must not leak into the session's list.
        let mut defs = DatabaseTools::definitions(
            allow_query_data,
            has_state_store,
            permit_candidate_writes,
            false,
        );
        if !advertises {
            // Read-only and never cannot prompt: everything write-shaped
            // stays hidden, not advertised. Bypass never lands here — its
            // advertised tools are usable, so the advertised-but-unusable
            // anti-pattern cannot return under it.
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
        // The host lane advertises under the same mode rule as every other
        // write-shaped member: ask-with-prompt or bypass. Read-only and
        // never never see the tool — hidden, not advertised — and an
        // uncomposed lane advertises nothing anywhere.
        if self.host.is_some() {
            defs.push(session_definitions::run_command());
        }
        defs
    }
}

#[cfg(test)]
#[path = "session_universe_tests.rs"]
mod tests;
