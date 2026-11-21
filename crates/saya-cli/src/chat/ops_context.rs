//! What one chat operation may touch, and the two typed requests it can raise.
//!
//! A context holds the session's own handles and nothing of the engine's: the
//! run's shared [`ModeCell`] — the one cell every decider reads, so a mode
//! stated here is in force from the next call — and whether this process's
//! configuration allows `bypass` at all.
//!
//! The operations themselves live next door ([`super::ops`]); this file holds
//! only what they may read and move, so a surface can tell what a command
//! touches without reading a command's body.
//!
//! A context change never happens here. [`RolloverRequest`] states what changed
//! and the session performs the rollover; the wording of a refusal is the mode
//! module's, so `/mode` cannot refuse the same request in two sentences.

use saya_agent::PermissionMode;

use crate::agent::tools::shell_policy::ModeCell;
use crate::cli::{ContractsCommand, InvestigationCommand, PreferencesCommand};
use crate::commands::recovery::RecoveryCommand;
use crate::interactive::session::commands::approval_name;
use crate::interactive::session::lifecycle::notes::NotesCommand;
use crate::interactive::session::permissions::mode;
use crate::slash::{ExportRequest, ModeRequest, ReportRequest};

/// The session's handles, as one operation sees them.
pub(crate) struct OpsContext {
    /// The run's mode cell: a switch stated here is what the next call reads.
    mode: ModeCell,
    /// Whether this process's configuration allows `bypass`.
    allow_bypass: bool,
}

impl OpsContext {
    /// A context over the run's cell, with `bypass` allowed or withheld.
    pub(crate) fn new(mode: ModeCell, allow_bypass: bool) -> Self {
        Self { mode, allow_bypass }
    }

    /// What the run's next call reads, in the mode module's own words.
    pub(crate) fn mode_words(&self) -> String {
        format!(
            "mode: the run's next call reads {}",
            approval_name(self.mode.get())
        )
    }

    /// States `next` on the run's cell and answers with the words the operator
    /// reads.
    ///
    /// The rule is the job adapter's ([`mode::stated_posture`]): a mode states
    /// its own consent posture, `plan` states `read-only` — the posture its own
    /// words name — and a switch never widens a cell already narrower than
    /// that. Refused for `bypass` while this process withholds it, and by a
    /// frozen cell, so a refused switch leaves the cell exactly as it found it.
    pub(crate) fn state_mode(&self, next: PermissionMode) -> Result<String, String> {
        if next == PermissionMode::Bypass && !self.allow_bypass {
            return Err(mode::BYPASS_REFUSAL.to_owned());
        }
        let (consent, _agent_mode) = next.decompose();
        let current = self.mode.get();
        let Some(posture) = mode::stated_posture(consent, current) else {
            return Ok(format!(
                "mode: {} — the run keeps its mode: {} is narrower than read-only, and a switch never widens it",
                mode::name(next),
                approval_name(current)
            ));
        };
        self.mode
            .set(posture)
            .map_err(|_| mode::FROZEN_REFUSAL.to_owned())?;
        Ok(format!(
            "mode: {} — the run's next call reads {}",
            mode::name(next),
            approval_name(posture)
        ))
    }
}

/// A context change that needs a rollover: the session performs it, and an
/// operation only ever states it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RolloverRequest {
    /// What changed.
    pub(crate) change: ContextChange,
    /// The value the command stated, where it stated one.
    pub(crate) value: Option<String>,
}

/// The change a command makes, which is what makes it a rollover.
///
/// The first six are context the next request is built from; the last four
/// empty or shrink the conversation itself, and each needs a fresh epoch for
/// the same reason — the seed of the epoch that follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContextChange {
    /// `/connect <profile>` — the primary database.
    Connect,
    /// `/include <profile>` — a secondary database.
    Include,
    /// `/exclude <profile>` — remove one.
    Exclude,
    /// `/provider <name>` — the model provider.
    Provider,
    /// `/model <name>` — the model.
    Model,
    /// `/privacy <on|off>` — the data-sharing gate.
    Privacy,
    /// `/clear` — a fresh conversation with no seed.
    Clear,
    /// `/compact` — a summary replaces the turns it covers.
    Compact,
    /// `/resume <id>` — a saved session's record.
    Resume,
    /// `/rotate` — a smaller successor keeps the summary and the tail.
    Rotate,
}

/// An operation on the session itself: never a turn of the run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SessionOp {
    /// `/mode`: state the mode on the run's cell; bare shows it. `/approvals`
    /// states the same operation through the one-release alias, so the note is
    /// the switch's, not a second command's.
    Mode(ModeRequest),
    /// `/allow`: seed the session's grant store.
    Allow(Vec<String>),
    /// `/thinking`: whether the transcript shows reasoning.
    Thinking(Option<bool>),
    /// `/columns`: the wide-table column filter.
    Columns(Option<String>),
    /// `/reload`: re-read the configuration between turns.
    Reload,
    /// `/setup`: hand the terminal to the guided setup.
    Setup,
    /// `/start`, `/follow-up`, `/job …` and the six child controls: a job
    /// beside the session's own run, never inside it.
    Job(crate::cli::CodingCommand),
    /// `/exit`, `/quit`.
    Exit,
}

/// Something to read or maintain that is not the run: the shared operation a
/// command names, for the surface to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReadOp {
    /// `/connections` — the profiles this configuration knows.
    Connections,
    /// `/provider` bare — the provider in force.
    Provider,
    /// `/model` bare — the model in force.
    Model,
    /// `/privacy` bare — the data-sharing gate.
    Privacy,
    /// `/schema [refresh]` — the schema view, or a refresh of it.
    Schema { refresh: bool },
    /// `/sql <query>` — one bounded read-only statement.
    Sql(String),
    /// `/export …` — write the last result.
    Export(ExportRequest),
    /// `/report …` — the Markdown report of the last result.
    Report(ReportRequest),
    /// `/chart [type] [path]` — a chart of the last result.
    Chart(String),
    /// `/explain [sql]` — the statement's plan.
    Explain(String),
    /// `/history` — the saved session ids.
    History,
    /// `/sessions` — the adapter's own listing.
    Sessions,
    /// `/sessions list` — the shared listing.
    SessionsList,
    /// `/sessions delete <id>` — the shared delete.
    SessionsDelete(String),
    /// `/preferences …` — the worktree's project preferences.
    Preferences(PreferencesCommand),
    /// `/contracts …` — the knowledge store.
    Contracts(ContractsCommand),
    /// `/investigation …` — saved investigations.
    Investigation(InvestigationCommand),
    /// `/runs [id]` — the run engine's own store.
    Runs(Option<String>),
    /// `/grants` — the session's grant store.
    Grants,
    /// `/context` — the instruction sources the next request would load.
    InstructionSources,
    /// `/doctor` — the configuration check.
    Doctor,
    /// `/usage` — the session's token totals.
    Usage,
    /// `/workspace` — the workspace binding.
    Workspace,
    /// `/help [command]` — the help text.
    Help(Option<String>),
    /// `/tasks [clear]` — the session task list.
    Tasks(Option<String>),
    /// `/notes …` — the session's own notes.
    Notes(NotesCommand),
    /// `/recovery …` — the session's per-edit recovery store.
    Recovery(RecoveryCommand),
}
