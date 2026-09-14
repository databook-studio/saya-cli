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
    /// The session journal, when this executor belongs to a session: a deny
    /// firing is journalled there before the refusal is relayed. `None` —
    /// test shapes — refuses without journaling.
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
            scratch,
            fetch,
            runner,
            host: None,
            deny: crate::interactive::session_deny::SessionDeny::default(),
            journal: None,
        }
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

    /// Composes with the host lane: the executor config plus the workspace
    /// root the child runs with as its cwd. The session surface is the only
    /// caller — runs never get the lane.
    pub(crate) fn with_host(
        mut self,
        config: saya_harness::host::HostConfig,
        workspace_root: std::path::PathBuf,
        cancellation: &CancellationToken,
    ) -> Self {
        self.host = Some(HostCommandMember {
            config,
            workspace_root,
            cancellation: cancellation.clone(),
        });
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
        // Deny first, at every program-named door: before grant lookup,
        // before the approval prompt, before bypass's auto-allow. Deny is a
        // structural refusal — it holds in every mode, bypass included.
        if matches!(name, "run_command" | "run_program") {
            let door = crate::interactive::session_deny::call_door(name);
            if let Some(program) = crate::interactive::session_deny::call_program(name, &arguments)
                && self.deny.contains(&program)
            {
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
            "scratch_sql" => match &self.scratch {
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
                Some(host) => host.execute(arguments).await,
                None => Err(ToolError::UnsupportedTool),
            },
            _ => self.database.execute(name, arguments).await,
        }
    }
}

/// The host lane's executor member: the composed config plus the workspace
/// root the child runs with as its cwd, carrying the turn's cancellation.
/// One program per call, typed argv — the runner's own contract reused.
struct HostCommandMember {
    config: saya_harness::host::HostConfig,
    workspace_root: std::path::PathBuf,
    cancellation: CancellationToken,
}

impl HostCommandMember {
    async fn execute(&self, arguments: serde_json::Value) -> Result<serde_json::Value, ToolError> {
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
            Some(serde_json::Value::Array(items)) => items
                .iter()
                .map(|item| {
                    item.as_str()
                        .map(str::to_owned)
                        .ok_or(ToolError::UnsupportedProperty)
                })
                .collect::<Result<_, _>>()?,
            Some(_) => return Err(ToolError::UnsupportedProperty),
        };
        let timeout_seconds = match object.get("timeout_seconds") {
            None => None,
            Some(serde_json::Value::Number(number)) => {
                Some(number.as_u64().ok_or(ToolError::UnsupportedProperty)?)
            }
            Some(_) => return Err(ToolError::UnsupportedProperty),
        };
        // The lane has no staging directory; the child's cwd pins to the
        // workspace root — a usability fact, never a bound.
        let _cwd = self.workspace_root.clone();
        let command = saya_harness::host::HostCommand::new(program.to_owned(), argv)
            .map_err(|error| ToolError::Runner(error.to_string()))?;
        let outcome = command
            .run(&self.config, timeout_seconds, &self.cancellation)
            .await
            .map_err(|error| ToolError::Runner(error.to_string()))?;
        serde_json::to_value(&outcome)
            .map_err(|_| ToolError::Runner("host outcome failed to render".into()))
    }
}
