//! The headless `saya run` surface — the CLI composition root for the run
//! engine.
//!
//! No policy lives here beyond one rule: a headless run states its scopes
//! up front or does not start. Scopes come from `--allow`, budgets from
//! `[jobs]` layered with `--budget` (never the environment — a run is
//! reproducible from its spec and config, plan G3); the goal comes from the
//! positional prompt. The engine (`saya-harness`) owns the state machine,
//! the journal, and the plan/episode drivers; this module builds the
//! collaborators the way `ask` builds a turn's, drives the steps blocking,
//! and maps the typed outcomes onto the documented exit codes.

mod assembly;
mod budget;
mod cancel;
mod claim;
mod drive;
mod exit;
mod files;
mod reads;
mod resume;
mod scopes;
mod start;

use crate::config::runtime::RuntimeConfig;
use crate::render::RenderFormat;
use saya_store::SqliteStateStore;
use saya_types::{PauseReason, RunEvent, RunId};
use std::path::PathBuf;

/// The `saya run` invocation the CLI parsed: the goal prompt, the approved
/// scopes, the budget overrides, and the optional management subcommand.
pub(super) struct RunInvocation {
    pub(super) prompt: Option<String>,
    pub(super) allow: Vec<String>,
    pub(super) budget: Vec<String>,
    pub(super) command: Option<crate::cli::RunCommand>,
}

pub(super) async fn run_command(
    invocation: RunInvocation,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    approval: saya_agent::ApprovalPolicy,
    state: &SqliteStateStore,
) -> Result<i32, Box<dyn std::error::Error>> {
    let RunInvocation {
        prompt,
        allow,
        budget,
        command,
    } = invocation;
    match command {
        Some(crate::cli::RunCommand::List) => reads::list(runtime, format, state).await,
        Some(crate::cli::RunCommand::Show { run_id }) => {
            reads::show(&run_id, runtime, format, state).await
        }
        Some(crate::cli::RunCommand::Log { run_id }) => {
            reads::log(&run_id, runtime, format, state).await
        }
        Some(crate::cli::RunCommand::Cancel { run_id }) => {
            cancel::cancel(&run_id, runtime, format, state).await
        }
        Some(crate::cli::RunCommand::Resume { run_id }) => {
            resume::resume(&run_id, runtime, format, approval, state).await
        }
        None => start::start(prompt, &allow, &budget, runtime, format, approval, state).await,
    }
}

/// The management surface the slash adapters share with the headless
/// `saya run` commands: one dispatcher onto the same `reads`/`cancel` paths,
/// so `/runs <id>` and `saya run show <id>` cannot drift — they are the same
/// call. No prompt, no scopes: a management read starts nothing.
pub async fn run_management(
    command: crate::cli::RunCommand,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    approval: saya_agent::ApprovalPolicy,
    state: &SqliteStateStore,
) -> Result<i32, Box<dyn std::error::Error>> {
    run_command(
        RunInvocation {
            prompt: None,
            allow: Vec::new(),
            budget: Vec::new(),
            command: Some(command),
        },
        runtime,
        format,
        approval,
        state,
    )
    .await
}

/// The runs root: `SAYA_RUNS_DIR` when set, the platform data home otherwise.
/// Resolved at call time, like every path the run surface touches.
pub(super) fn runs_dir() -> PathBuf {
    saya_harness::paths::default_runs_dir()
}

/// Parses a run-id argument. A malformed id is a usage error, so an untrusted
/// string never becomes a path component.
pub(super) fn parse_run_id(raw: &str) -> Result<RunId, String> {
    RunId::parse(raw)
        .map_err(|_| "run id must be non-empty and use only letters, digits, '-', '_'".to_string())
}

/// A fresh run id: milliseconds plus pid — unique on one machine, and inside
/// the shape [`saya_types::RunId`] allows for a path component.
pub(super) fn new_run_id() -> RunId {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis())
        .unwrap_or_default();
    RunId::parse(&format!("r{millis}-{}", std::process::id()))
        .expect("generated run id satisfies the run-id shape")
}

/// The pause reason the journal last recorded, for messages that name it.
pub(super) fn last_pause(journal: &saya_harness::journal::Journal) -> Option<PauseReason> {
    match journal.rebuild() {
        Ok(state) => match state.last {
            Some(RunEvent::Paused { reason }) => Some(reason),
            _ => None,
        },
        Err(_) => None,
    }
}

/// Where the journal says a run stands — the full lifecycle position a cancel
/// needs. `None` when the journal holds no run.
pub(super) fn journal_position(
    state: &saya_harness::journal::JournalState,
) -> Option<saya_harness::engine::RunState> {
    use saya_harness::engine::RunState;
    if !state.started {
        return None;
    }
    if !state.plan_approved {
        return Some(RunState::Planned);
    }
    match state.last {
        Some(RunEvent::Completed) => Some(RunState::Completed),
        Some(RunEvent::Failed { .. }) => Some(RunState::Failed),
        Some(RunEvent::Cancelled) => Some(RunState::Cancelled),
        Some(RunEvent::Paused { .. }) => Some(RunState::Paused),
        _ if !state.steps.is_empty() => Some(RunState::Executing),
        _ => Some(RunState::Approved),
    }
}

/// The journal's terminal tail for the exit mapping after a mid-resume
/// failure: the durable authority on where the run ended. A non-terminal
/// tail (mid-flight steps, no lifecycle event) reads as `executing`.
pub(super) fn journal_tail(
    journal: &saya_harness::journal::Journal,
) -> (
    saya_harness::engine::RunState,
    Option<saya_types::RunFailureCode>,
) {
    use saya_harness::engine::RunState;
    match journal.rebuild() {
        Ok(state) => match state.last {
            Some(RunEvent::Completed) => (RunState::Completed, None),
            Some(RunEvent::Failed { code }) => (RunState::Failed, Some(code)),
            Some(RunEvent::Cancelled) => (RunState::Cancelled, None),
            Some(RunEvent::Paused { .. }) => (RunState::Paused, None),
            _ => (RunState::Executing, None),
        },
        Err(_) => (RunState::Executing, None),
    }
}
