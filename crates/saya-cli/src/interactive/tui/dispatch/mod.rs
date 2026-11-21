//! Routes a submitted input line to command handling and renders the outcome
//! into the transcript. Slash commands reuse the shared `state.apply` logic;
//! results that would normally print to stdout are captured via `render_event`
//! and pushed into the transcript instead.

mod chart;
mod grants;
mod outcome;
mod query;
mod query_source_guard;
mod run;
mod session_actions;

#[cfg(test)]
#[path = "../export_mode_tests.rs"]
mod export_mode_tests;

#[cfg(test)]
#[path = "../../session_context_key_tests.rs"]
mod session_context_key_tests;

#[cfg(test)]
mod chain_tests;

#[cfg(test)]
#[path = "query_source_tests.rs"]
mod query_source_tests;

pub(crate) use outcome::Dispatch;

use super::transcript::{BlockKind, Transcript};
use super::types::{CapturedResult, LastQuery};
use crate::agent::tools::shell_policy::ModeCell;
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session::commands::SessionAction;
use crate::interactive::session::runtime::SessionRuntime;
use crate::interactive::session::state::SessionState;
use crate::render::RenderFormat;
use crate::slash::{SlashCommand, parse_slash_command};
use saya_store::FsSessionStore;

/// Dispatches one submitted line, mutating `state` and appending to `transcript`.
/// A non-command line returns `Dispatch::Agent` for the caller to stream.
/// `captured` is the session's ephemeral result capture, read by the export
/// snapshot (a snapshot export is decided and written here, with no query).
/// `job` is the job the panel is running, when one is: `/mode` states the mode
/// on it through the same switch it states the session's own.
#[allow(clippy::too_many_arguments)]
pub(crate) fn dispatch(
    line: &str,
    transcript: &mut Transcript,
    profiles: &[String],
    state: &mut SessionState,
    runtime: &RuntimeConfig,
    store: &FsSessionStore,
    state_db: &saya_store::SqliteStateStore,
    format: RenderFormat,
    last_query: &mut Option<LastQuery>,
    captured: &Option<CapturedResult>,
    capture_gap: Option<super::capture_agent::CaptureGap>,
    session: &mut SessionRuntime,
    job: Option<&ModeCell>,
) -> Dispatch {
    // In the TUI, /sessions opens an interactive picker rather than a text list.
    if line.trim() == "/sessions" {
        return Dispatch::OpenSessionPicker;
    }
    // A user-authored command (`<config home>/commands/`, `.saya/commands/`)
    // resolves BEFORE the built-in parser and its expansion is never re-parsed
    // as a slash command: a command file — the project's is untrusted input —
    // can only ever produce a user message, never a config, tool, or
    // permission change. The transcript keeps the typed line; the agent
    // receives the expansion.
    match state.user_commands.route(line) {
        crate::slash::user_commands::UserLine::Builtin => {}
        crate::slash::user_commands::UserLine::Prompt(prompt) => return Dispatch::Agent(prompt),
        crate::slash::user_commands::UserLine::Refused(message) => {
            transcript.push(BlockKind::Error, message);
            transcript.scroll_to_bottom();
            return Dispatch::Handled;
        }
    }
    // Bare /investigations opens the searchable investigation picker rather
    // than the text list; `/investigation list` and any flagged
    // /investigations tail (`--list`, `--limit`, `--offset`) keep it.
    if super::dispatch_investigation::bare_investigations(line) {
        return Dispatch::OpenInvestigationPicker;
    }
    let mut result = Dispatch::Handled;
    let parsed = parse_slash_command(line);
    match parsed {
        Err(error) => transcript.push(BlockKind::Error, error.to_string()),
        Ok(Some(command)) => match command {
            SlashCommand::Coding(command) => return Dispatch::Coding(command),
            SlashCommand::Columns(arg) => {
                result = Dispatch::SetColumns(arg);
            }
            command => {
                // The answering context before and after the command decides
                // whether this command changed it: a changed key retires the
                // question authority once the command's message is produced,
                // an unchanged key leaves a pending clarification untouched.
                let before = state.context_key();
                match state.apply(command, profiles) {
                    SessionAction::Message(message) => {
                        transcript.push(BlockKind::System, message);
                        state.abandon_question_when_context_changed(&before, session);
                    }
                    SessionAction::Error(message) => transcript.push(BlockKind::Error, message),
                    // One concern per helper: each takes `&mut Option<SessionAction>`
                    // and takes the action only when it owns the arm — an owned arm
                    // always reports handled (`Some`), an unowned one is left in
                    // place for the next helper. What no helper owns is decided
                    // below, never a panic.
                    action => {
                        let mut action = Some(action);
                        if let Some(outcome) = session_actions::apply_session_action(
                            &mut action,
                            transcript,
                            state,
                            runtime,
                            store,
                            state_db,
                            format,
                            session,
                            last_query,
                            job,
                        ) {
                            result = outcome;
                        } else if let Some(outcome) =
                            grants::apply_grant_action(&mut action, transcript, runtime, session)
                        {
                            result = outcome;
                        } else if let Some(outcome) = query::apply_query_actions(
                            &mut action,
                            transcript,
                            state,
                            last_query,
                            captured,
                            capture_gap,
                            runtime,
                        ) {
                            result = outcome;
                        } else if let Some(outcome) = run::apply_run_action(
                            &mut action,
                            transcript,
                            runtime,
                            state_db,
                            format,
                            session,
                        ) {
                            result = outcome;
                        } else if let Some(action) = action.take() {
                            match action {
                                SessionAction::Schema(_) => transcript.push(
                                    BlockKind::System,
                                    "Schema view is available in headless mode; TUI rendering is coming next.",
                                ),
                                SessionAction::Compact => {
                                    result = Dispatch::Compact;
                                }
                                SessionAction::Agent(_) | SessionAction::Cancelled => {}
                                SessionAction::NotImplemented(feature) => transcript.push(
                                    BlockKind::System,
                                    format!("Not implemented: {feature}"),
                                ),
                                SessionAction::Exit => result = Dispatch::Quit,
                                other => transcript.push(
                                    BlockKind::System,
                                    format!("Not supported here: {other:?}"),
                                ),
                            }
                        }
                    }
                }
            }
        },
        Ok(None) => result = Dispatch::Agent(line.to_string()),
    }
    transcript.scroll_to_bottom();
    result
}
