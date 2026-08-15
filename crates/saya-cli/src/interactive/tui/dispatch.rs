//! Routes a submitted input line to command handling and renders the outcome
//! into the transcript. Slash commands reuse the shared `state.apply` logic;
//! results that would normally print to stdout are captured via `render_event`
//! and pushed into the transcript instead.

use super::dispatch_actions::{list_sessions, resume, run_chart, run_explain, run_export, run_sql};
use super::dispatch_contracts::{run_contracts, run_preferences};
use super::transcript::{BlockKind, Transcript};
use super::types::LastQuery;
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session_commands::SessionAction;
use crate::interactive::session_state::SessionState;
use crate::render::RenderFormat;
use crate::slash::parse_slash_command;
use saya_store::FsSessionStore;

/// Outcome of dispatching one line.
pub(crate) enum Dispatch {
    /// A command was handled synchronously; keep looping.
    Handled,
    /// The session should exit.
    Quit,
    /// The line is a prompt for the agent; the caller starts streaming it.
    Agent(String),
    /// Open the interactive session picker.
    OpenSessionPicker,
}

/// Dispatches one submitted line, mutating `state` and appending to `transcript`.
/// A non-command line returns `Dispatch::Agent` for the caller to stream.
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
) -> Dispatch {
    // In the TUI, /sessions opens an interactive picker rather than a text list.
    if line.trim() == "/sessions" {
        return Dispatch::OpenSessionPicker;
    }
    let mut result = Dispatch::Handled;
    match parse_slash_command(line) {
        Err(error) => transcript.push(BlockKind::Error, error.to_string()),
        Ok(Some(command)) => match state.apply(command, profiles) {
            SessionAction::Message(message) => transcript.push(BlockKind::System, message),
            SessionAction::Error(message) => transcript.push(BlockKind::Error, message),
            SessionAction::History => list_sessions(transcript, store),
            SessionAction::Resume(id) => resume(transcript, state, store, &id),
            SessionAction::Sql(sql) => {
                run_sql(transcript, state, runtime, format, &sql, last_query)
            }
            SessionAction::Contracts(command) => {
                run_contracts(transcript, state, runtime, state_db, format, &command)
            }
            SessionAction::Preferences(command) => {
                run_preferences(transcript, state, runtime, state_db, format, &command)
            }
            SessionAction::Export(path) => {
                run_export(transcript, runtime, state, last_query, &path)
            }
            SessionAction::Chart(args) => run_chart(transcript, runtime, state, last_query, &args),
            SessionAction::Explain(arg) => {
                run_explain(transcript, runtime, state, last_query, &arg)
            }
            SessionAction::Schema(_) => transcript.push(
                BlockKind::System,
                "Schema view is available in headless mode; TUI rendering is coming next.",
            ),
            SessionAction::Agent(_) | SessionAction::Cancelled => {}
            SessionAction::NotImplemented(feature) => {
                transcript.push(BlockKind::System, format!("Not implemented: {feature}"))
            }
            SessionAction::Exit => result = Dispatch::Quit,
        },
        Ok(None) => result = Dispatch::Agent(line.to_string()),
    }
    transcript.scroll_to_bottom();
    result
}
