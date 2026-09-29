//! Routes a submitted input line to command handling and renders the outcome
//! into the transcript. Slash commands reuse the shared `state.apply` logic;
//! results that would normally print to stdout are captured via `render_event`
//! and pushed into the transcript instead.

mod approvals;
mod chart;
mod grants;
mod outcome;
mod query;
mod run;
mod session_actions;

#[cfg(test)]
#[path = "../export_mode_tests.rs"]
mod export_mode_tests;

#[cfg(test)]
mod chain_tests;

pub(crate) use outcome::Dispatch;

use super::super::session_runtime::SessionRuntime;
use super::transcript::{BlockKind, Transcript};
use super::types::{CapturedResult, LastQuery};
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session_commands::SessionAction;
use crate::interactive::session_state::SessionState;
use crate::render::RenderFormat;
use crate::slash::{SlashCommand, parse_slash_command};
use saya_store::FsSessionStore;

/// Dispatches one submitted line, mutating `state` and appending to `transcript`.
/// A non-command line returns `Dispatch::Agent` for the caller to stream.
/// `captured` is the session's ephemeral result capture, read by the export
/// snapshot (a snapshot export is decided and written here, with no query).
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
) -> Dispatch {
    // In the TUI, /sessions opens an interactive picker rather than a text list.
    if line.trim() == "/sessions" {
        return Dispatch::OpenSessionPicker;
    }
    // Bare /investigations opens the searchable investigation picker rather
    // than the text list; `/investigation list` and any flagged
    // /investigations tail (`--list`, `--limit`, `--offset`) keep it.
    if super::dispatch_investigation::bare_investigations(line) {
        return Dispatch::OpenInvestigationPicker;
    }
    let mut result = Dispatch::Handled;
    // A mode change through `/approvals` carries the activation line with
    // it: under bypass the no-euphemism wording, the staged interpreter
    // facts, and the probe's verdict — said where the mode is set. The mode
    // before the command decides whether this command newly activated
    // bypass: only then is a consent recorded in the journal; a
    // re-statement over an already-bypass session records none, and a
    // failed journal write is said, not silent.
    let before_mode = state.approval_mode.clone();
    let parsed = parse_slash_command(line);
    let approvals_set = matches!(parsed, Ok(Some(SlashCommand::Approvals(Some(_)))));
    match parsed {
        Err(error) => transcript.push(BlockKind::Error, error.to_string()),
        Ok(Some(command)) => match command {
            SlashCommand::Columns(arg) => {
                result = Dispatch::SetColumns(arg);
            }
            command => match state.apply(command, profiles) {
                SessionAction::Message(message) => transcript.push(BlockKind::System, message),
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
            },
        },
        Ok(None) => result = Dispatch::Agent(line.to_string()),
    }
    approvals::report_approvals_set(
        approvals_set,
        &before_mode,
        transcript,
        state,
        runtime,
        session,
    );
    transcript.scroll_to_bottom();
    result
}
