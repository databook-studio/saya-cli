//! Routes a submitted input line to command handling and renders the outcome
//! into the transcript. Slash commands reuse the shared `state.apply` logic;
//! results that would normally print to stdout are captured via `render_event`
//! and pushed into the transcript instead.

use super::exec;
use super::transcript::{BlockKind, Transcript};
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session_commands::SessionAction;
use crate::interactive::session_resume::{SessionDefaults, block_on, resume_session};
use crate::interactive::session_state::SessionState;
use crate::render::{RenderFormat, TerminalEvent, render_event};
use crate::slash::parse_slash_command;
use saya_store::{FsSessionStore, SessionStore};

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
pub(crate) fn dispatch(
    line: &str,
    transcript: &mut Transcript,
    profiles: &[String],
    state: &mut SessionState,
    runtime: &RuntimeConfig,
    store: &FsSessionStore,
    format: RenderFormat,
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
            SessionAction::Sql(sql) => run_sql(transcript, state, runtime, format, &sql),
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
    let _ = block_on(store.save(state.redacted()));
    transcript.scroll_to_bottom();
    result
}

/// Lists saved sessions (shared with the `/history` command).
fn list_sessions(transcript: &mut Transcript, store: &FsSessionStore) {
    match block_on(store.history()) {
        Ok(entries) if entries.is_empty() => {
            transcript.push(BlockKind::System, "No saved sessions.")
        }
        Ok(entries) => {
            let body = entries
                .into_iter()
                .map(|entry| format!("{}\t{}", entry.id, entry.modified_unix_ms))
                .collect::<Vec<_>>()
                .join("\n");
            transcript.push(BlockKind::System, body);
        }
        Err(error) => transcript.push(BlockKind::Error, error.to_string()),
    }
}

/// Loads a saved session by id and makes it active, falling back to the current
/// session's settings for any fields the saved copy lacks.
fn resume(transcript: &mut Transcript, state: &mut SessionState, store: &FsSessionStore, id: &str) {
    let defaults = SessionDefaults {
        provider: state.provider.clone(),
        model: state.model.clone(),
        allow_data_sharing: state.allow_data_sharing,
        approval_mode: state.approval_mode.clone(),
    };
    match resume_session(store, id, &defaults) {
        Ok(Some(loaded)) => {
            *state = loaded;
            transcript.push(BlockKind::System, format!("Resumed session {id}"));
        }
        Ok(None) => transcript.push(BlockKind::Error, format!("Session not found: {id}")),
        Err(error) => transcript.push(BlockKind::Error, error.to_string()),
    }
}

/// Runs raw SQL and pushes the rendered result (or error) into the transcript.
fn run_sql(
    transcript: &mut Transcript,
    state: &SessionState,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    sql: &str,
) {
    let event = block_on(exec::run_sql(runtime, state.profile.as_deref(), sql));
    match event {
        TerminalEvent::QueryResult { result } => {
            transcript.push(BlockKind::Tool, super::table::format_table(&result));
        }
        TerminalEvent::Error { message } => {
            transcript.push(BlockKind::Error, message);
        }
        other => {
            let rendered = render_event(&other, format);
            transcript.push(BlockKind::System, rendered.stdout.trim_end().to_string());
        }
    }
}
