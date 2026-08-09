//! Routes a submitted input line to command handling and renders the outcome
//! into the transcript. Slash commands reuse the shared `state.apply` logic;
//! results that would normally print to stdout are captured via `render_event`
//! and pushed into the transcript instead.

use super::exec;
use super::transcript::{BlockKind, Transcript};
use super::types::LastQuery;
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session_commands::SessionAction;
use crate::interactive::session_resume::{SessionDefaults, block_on, resume_session};
use crate::interactive::session_state::SessionState;
use crate::render::{RenderFormat, TerminalEvent, render_event};
use crate::slash::parse_slash_command;
use saya_store::{FsSessionStore, SessionStore};
use std::path::Path;

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
    last_query: &mut Option<LastQuery>,
) {
    let event = block_on(exec::run_sql(runtime, state.profile.as_deref(), sql));
    match event {
        TerminalEvent::QueryResult { result } => {
            *last_query = Some(LastQuery {
                sql: sql.to_string(),
                connection: state.profile.clone(),
            });
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

fn run_export(
    transcript: &mut Transcript,
    runtime: &RuntimeConfig,
    state: &SessionState,
    last_query: &Option<LastQuery>,
    path: &str,
) {
    let Some(lq) = last_query.as_ref() else {
        transcript.push(
            BlockKind::System,
            "Nothing to export yet — run a query first.",
        );
        return;
    };
    let target = lq.connection.as_deref().or(state.profile.as_deref());
    let event = block_on(exec::run_sql(runtime, target, &lq.sql));
    match event {
        TerminalEvent::QueryResult { result } => {
            match super::export::write_result(&result, Path::new(path)) {
                Ok(n) => {
                    let mut msg = format!("Exported {n} row(s) to {path}");
                    if result.truncated {
                        msg.push_str(" (result was truncated)");
                    }
                    transcript.push(BlockKind::System, msg);
                }
                Err(msg) => transcript.push(BlockKind::Error, msg),
            }
        }
        TerminalEvent::Error { message } => {
            transcript.push(BlockKind::Error, message);
        }
        _ => {}
    }
}

fn run_chart(
    transcript: &mut Transcript,
    runtime: &RuntimeConfig,
    state: &SessionState,
    last_query: &Option<LastQuery>,
    args: &str,
) {
    let Some(lq) = last_query.as_ref() else {
        transcript.push(
            BlockKind::System,
            "Nothing to chart yet — run a query first.",
        );
        return;
    };
    // Parse "[type] [path]": if the first token is a known chart kind, use it; the remaining
    // token (if any) is the output path.
    let mut tokens = args.split_whitespace();
    let (kind, path_arg) = match tokens.next() {
        Some(tok) => match super::chart::ChartKind::parse(tok) {
            Some(k) => (Some(k), tokens.next()),
            None => (None, Some(tok)),
        },
        None => (None, None),
    };
    let target = lq.connection.as_deref().or(state.profile.as_deref());
    let result = match block_on(exec::run_sql(runtime, target, &lq.sql)) {
        TerminalEvent::QueryResult { result } => result,
        TerminalEvent::Error { message } => {
            transcript.push(BlockKind::Error, message);
            return;
        }
        _ => return,
    };
    let mut spec = super::chart::suggest_spec(&result);
    if let Some(k) = kind {
        spec.kind = k;
    }
    let html = match super::chart::render_html(&result, &spec) {
        Ok(html) => html,
        Err(msg) => {
            transcript.push(BlockKind::System, msg);
            return;
        }
    };
    let path = match path_arg {
        Some(p) => std::path::PathBuf::from(p),
        None => std::env::temp_dir().join("saya-chart.html"),
    };
    if let Err(msg) = super::chart::write_html(&html, &path) {
        transcript.push(BlockKind::Error, msg);
        return;
    }
    let mut note = format!("Chart written to {}", path.display());
    match super::chart::open_file(&path) {
        Ok(()) => note.push_str(" (opening in your browser)"),
        Err(e) => note.push_str(&format!(" — open it manually ({e})")),
    }
    transcript.push(BlockKind::System, note);
}

/// Runs EXPLAIN for the provided SQL query or the last executed query.
fn run_explain(
    transcript: &mut Transcript,
    runtime: &RuntimeConfig,
    state: &SessionState,
    last_query: &Option<LastQuery>,
    arg: &str,
) {
    // Choose SQL + connection: explicit arg uses the active profile; empty arg
    // reuses the last query and its connection.
    let (sql, connection) = if !arg.trim().is_empty() {
        (arg.trim().to_string(), None)
    } else if let Some(lq) = last_query.as_ref() {
        (lq.sql.clone(), lq.connection.clone())
    } else {
        transcript.push(
            BlockKind::System,
            "Nothing to explain — run a query first, or pass SQL: /explain SELECT ...",
        );
        return;
    };
    let trimmed = sql.trim().trim_end_matches(';').trim();
    let explain_sql = format!("EXPLAIN {trimmed}");
    let target = connection.as_deref().or(state.profile.as_deref());
    match block_on(exec::run_sql(runtime, target, &explain_sql)) {
        TerminalEvent::QueryResult { result } => {
            transcript.push(BlockKind::Tool, super::table::format_plan(&result));
        }
        TerminalEvent::Error { message } => transcript.push(BlockKind::Error, message),
        _ => {}
    }
}
