//! The run-tool composite executor: the four fixed harness tool names route
//! to their member executors, everything else falls through to the shared
//! `DatabaseTools` — whose typed `UnsupportedTool` refusal for unknown names
//! makes the fall-through total and predictable. One composite serves both
//! surfaces that carry the run tool members (a run's per-step toolsets and an
//! interactive session), so the dispatch never forks.

use std::sync::Arc;

use async_trait::async_trait;
use saya_agent::{ToolError, ToolExecutor};

use super::DatabaseTools;
use super::host_argv::validate_host_json_argv;
use saya_agent::CancellationToken;
use saya_harness::fetch::FetchTools;
use saya_harness::runner::RunProgram;
use saya_harness::scratch::ScratchSql;

/// The composite executor. Each optional member is present only where its
/// surface composed it: a surface without a member refuses that member's
/// names as unknown tools, before any permit is consulted — the
/// hidden-not-advertised discipline applied at dispatch, so an advertised
/// tool and its executor cannot drift apart.
pub(crate) struct RunTools {
    database: Arc<DatabaseTools>,
    chart_save_permit: bool,
    /// The scratch database, present only where scratch was composed. Its
    /// absence refuses `scratch_sql` as an unknown tool.
    scratch: Option<Arc<ScratchSql>>,
    /// The fetch member, present only where fetch was composed. Its absence
    /// refuses both fetch names as unknown tools.
    fetch: Option<Arc<FetchTools>>,
    /// The runner member, present only where a proven spawn exists. Its
    /// absence refuses `run_program` as an unknown tool — an unproven host
    /// never has the tool at all.
    runner: Option<Arc<RunProgram>>,
    /// The host lane, present only where the launch stated it and a
    /// workspace root bound. Its absence refuses `run_command` as an
    /// unknown tool.
    host: Option<HostCommandMember>,
    /// The session deny list: bare program names refused before grant,
    /// prompt, and bypass at every program-named door. Runs never carry it
    /// (deny is session-shaped); an empty list refuses nothing.
    deny: crate::interactive::session_deny::SessionDeny,
    /// The session's live task list, present only where a session composed
    /// it. Its absence refuses `tasks_set` as an unknown tool — a run has
    /// no session list to write.
    tasks: Option<crate::interactive::session_tasks::SessionTasks>,
    /// The session journal, when this executor belongs to a session: a deny
    /// firing is journalled there before the refusal is relayed, and a host
    /// call before the child spawns. `None` — test shapes — refuses without
    /// journaling.
    journal: Option<std::sync::Arc<saya_store::SessionJournal>>,
}

impl RunTools {
    /// Composes the members into the one executor. Every surface that
    /// advertises a harness tool dispatches through this constructor — there
    /// is no second assembly path.
    pub(crate) fn compose(
        database: Arc<DatabaseTools>,
        scratch: Option<Arc<ScratchSql>>,
        fetch: Option<Arc<FetchTools>>,
        runner: Option<Arc<RunProgram>>,
    ) -> Self {
        Self {
            database,
            chart_save_permit: false,
            scratch,
            fetch,
            runner,
            host: None,
            deny: crate::interactive::session_deny::SessionDeny::default(),
            tasks: None,
            journal: None,
        }
    }

    /// Grants this composite permission to save charts into its workspace.
    pub(crate) fn with_chart_save_permit(mut self, permit: bool) -> Self {
        self.chart_save_permit = permit;
        self
    }

    /// Composes with the session deny list: refused at the `run_command`,
    /// `run_program`, and interpreter doors before anything else. The
    /// session surface is the only caller — runs never get the list.
    pub(crate) fn with_session_deny(
        mut self,
        deny: crate::interactive::session_deny::SessionDeny,
    ) -> Self {
        self.deny = deny;
        self
    }

    /// Composes with the session's live task list: `tasks_set` writes
    /// through it. The session surface is the only caller — runs never get
    /// the cell, and refuse the name as unknown.
    pub(crate) fn with_tasks(
        mut self,
        tasks: crate::interactive::session_tasks::SessionTasks,
    ) -> Self {
        self.tasks = Some(tasks);
        self
    }

    /// Composes with the session journal: a deny firing is written there
    /// before the refusal is relayed. Wired by `SessionUniverse`'s
    /// journal-carrying executor; the deny tests attach it directly.
    pub(crate) fn with_session_journal(
        mut self,
        journal: std::sync::Arc<saya_store::SessionJournal>,
    ) -> Self {
        self.journal = Some(journal);
        self
    }

    /// Composes with the host lane: the executor config, whose workspace
    /// root the child runs with as its cwd. The session surface is the only
    /// caller — runs never get the lane.
    pub(crate) fn with_host(
        mut self,
        config: saya_harness::host::HostConfig,
        cancellation: &CancellationToken,
    ) -> Self {
        self.host = Some(HostCommandMember {
            config,
            cancellation: cancellation.clone(),
            host_ran: None,
        });
        self
    }

    /// Shares the session's host-ran flag with the host member: set after a
    /// host call settles, read by later `run_program` prompts (§3 rule 6).
    /// A no-op without the lane — the flag rides the member, not the
    /// composite.
    pub(crate) fn with_host_ran(
        mut self,
        host_ran: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        if let Some(host) = self.host.as_mut() {
            host.host_ran = Some(host_ran);
        }
        self
    }
}

#[async_trait]
impl ToolExecutor for RunTools {
    async fn execute(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        if name == "render_chart" && arguments.get("save_to").is_some() && !self.chart_save_permit {
            return Err(ToolError::WorkspaceWrite(
                "workspace-write is not permitted for this step".into(),
            ));
        }
        // Deny first, at every program-named door: before grant lookup,
        // before the approval prompt, before bypass's auto-allow. Deny is a
        // structural refusal — it holds in every mode, bypass included.
        if matches!(name, "run_command" | "run_program") {
            let door = crate::interactive::session_deny::call_door(name);
            if let Some(program) = crate::interactive::session_deny::call_program(name, &arguments)
                && self.deny.contains(&program)
            {
                // The deny journal line must not materialise unbounded argv
                // either: validate the raw JSON first, then carry at most the
                // admitted bound into the owned strings the journal redacts.
                if name == "run_command"
                    && let Some(serde_json::Value::Array(items)) = arguments.get("args")
                {
                    validate_host_json_argv(items)?;
                }
                let argv: Vec<String> = arguments
                    .get("args")
                    .and_then(serde_json::Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| item.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
                if let Some(journal) = self.journal.as_ref() {
                    let _ = journal.command_denied(&program, &argv, door);
                }
                return Err(ToolError::Runner(
                    crate::interactive::session_deny::denied_refusal(&program),
                ));
            }
        }
        match name {
            "scratch_sql" | "scratch_import" => match &self.scratch {
                Some(scratch) => scratch.execute(name, arguments).await,
                None => Err(ToolError::UnsupportedTool),
            },
            "http_fetch" | "http_download" => match &self.fetch {
                Some(fetch) => fetch.execute(name, arguments).await,
                None => Err(ToolError::UnsupportedTool),
            },
            "run_program" => match &self.runner {
                Some(runner) => runner.execute(name, arguments).await,
                None => Err(ToolError::UnsupportedTool),
            },
            "run_command" => match &self.host {
                Some(host) => {
                    host.execute_with_journal(arguments, self.journal.as_deref())
                        .await
                }
                None => Err(ToolError::UnsupportedTool),
            },
            // The session task list: whole-list replace against the shared
            // cell. Absent outside a session — a run has no session list —
            // where the name refuses as unknown, the hidden-not-advertised
            // discipline at dispatch.
            "tasks_set" => match &self.tasks {
                Some(tasks) => {
                    crate::interactive::session_tasks::TasksSet::new(tasks.clone())
                        .execute(name, arguments)
                        .await
                }
                None => Err(ToolError::UnsupportedTool),
            },
            _ => self.database.execute(name, arguments).await,
        }
    }
}

/// The host lane's executor member: the composed config, whose workspace
/// root the child runs with as its cwd, carrying the turn's cancellation.
/// One program per call, typed argv — the runner's own contract reused.
struct HostCommandMember {
    config: saya_harness::host::HostConfig,
    cancellation: CancellationToken,
    /// The session's host-ran flag, set after a host call settles so later
    /// `run_program` prompts gain the staged-binary integrity line.
    host_ran: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

impl HostCommandMember {
    /// Runs one host call: journals `session-command` before the child
    /// spawns — consent-before-action for the one lane where it matters
    /// most — then resolves, builds, and spawns. A failed journal write
    /// changes no consent: the call still runs, like every other journaling
    /// site's posture (the consent stands, the audit line is missing).
    /// Test shapes without a journal reach the same path with `None`: parse
    /// and spawn with no journal line.
    async fn execute_with_journal(
        &self,
        arguments: serde_json::Value,
        journal: Option<&saya_store::SessionJournal>,
    ) -> Result<serde_json::Value, ToolError> {
        let parsed = self.parse(&arguments)?;
        if let Some(journal) = journal {
            let _ = journal.command(&parsed.program, &parsed.argv, "run_command");
        }
        self.spawn(parsed).await
    }

    fn parse(&self, arguments: &serde_json::Value) -> Result<ParsedHostCall, ToolError> {
        let object = arguments.as_object().ok_or(ToolError::ArgumentsNotObject)?;
        for key in object.keys() {
            if !matches!(key.as_str(), "program" | "args" | "timeout_seconds") {
                return Err(ToolError::UnsupportedProperty);
            }
        }
        let program = object
            .get("program")
            .and_then(serde_json::Value::as_str)
            .ok_or(ToolError::UnsupportedProperty)?;
        let argv: Vec<String> = match object.get("args") {
            None => Vec::new(),
            Some(serde_json::Value::Array(items)) => {
                // Admission before cloning and before journalling: the bound
                // runs on the borrowed JSON, so an oversized array refuses
                // without materialising an owned copy or a journal line.
                validate_host_json_argv(items)?;
                items
                    .iter()
                    .map(|item| {
                        item.as_str()
                            .map(str::to_owned)
                            .ok_or(ToolError::UnsupportedProperty)
                    })
                    .collect::<Result<_, _>>()?
            }
            Some(_) => return Err(ToolError::UnsupportedProperty),
        };
        let timeout_seconds = match object.get("timeout_seconds") {
            None => None,
            Some(serde_json::Value::Number(number)) => {
                Some(number.as_u64().ok_or(ToolError::UnsupportedProperty)?)
            }
            Some(_) => return Err(ToolError::UnsupportedProperty),
        };
        Ok(ParsedHostCall {
            program: program.to_owned(),
            argv,
            timeout_seconds,
        })
    }

    async fn spawn(&self, parsed: ParsedHostCall) -> Result<serde_json::Value, ToolError> {
        // The lane has no staging directory; the child's cwd pins to the
        // workspace root through the config — `HostConfig::workspace_root`
        // applied as `current_dir` in `HostCommand::run` — so the prompt's
        // `cwd: pinned to <root>` line is the executor's own fact.
        let command = saya_harness::host::HostCommand::new(parsed.program, parsed.argv)
            .map_err(|error| ToolError::Runner(error.to_string()))?;
        let result = command
            .run(&self.config, parsed.timeout_seconds, &self.cancellation)
            .await
            .map_err(|error| ToolError::Runner(error.to_string()));
        // A settled host call — success or child failure — means a host
        // child ran: later `run_program` prompts gain the integrity line. A
        // refusal (validation, resolution) ran nothing, so the flag stays.
        if result.is_ok()
            && let Some(host_ran) = self.host_ran.as_ref()
        {
            host_ran.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        let outcome = result?;
        serde_json::to_value(&outcome)
            .map_err(|_| ToolError::Runner("host outcome failed to render".into()))
    }
}

/// One parsed host call: the bare program, its typed argv, and the call's
/// own timeout narrowing.
struct ParsedHostCall {
    program: String,
    argv: Vec<String>,
    timeout_seconds: Option<u64>,
}
