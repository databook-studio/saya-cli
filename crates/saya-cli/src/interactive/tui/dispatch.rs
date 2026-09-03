//! Routes a submitted input line to command handling and renders the outcome
//! into the transcript. Slash commands reuse the shared `state.apply` logic;
//! results that would normally print to stdout are captured via `render_event`
//! and pushed into the transcript instead.

use super::dispatch_actions::{list_sessions, resume};
use super::dispatch_contracts::run_contracts;
use super::transcript::{BlockKind, Transcript};
use super::types::LastQuery;
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session_commands::SessionAction;
use crate::interactive::session_state::SessionState;
use crate::render::RenderFormat;
use crate::slash::{SlashCommand, parse_slash_command};
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
    /// A SQL-backed command runs on a worker thread; the caller stores the
    /// receiver and applies [`sql_task::complete`] when it finishes.
    SqlTask(super::sql_task::SqlTask),
    /// `/columns` — set which columns wide result tables show. Handled by the
    /// caller, which owns the view state on `App`.
    SetColumns(Option<String>),
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
        Ok(Some(command)) => match command {
            SlashCommand::Columns(arg) => {
                result = Dispatch::SetColumns(arg);
            }
            command => match state.apply(command, profiles) {
                SessionAction::Message(message) => transcript.push(BlockKind::System, message),
                SessionAction::Error(message) => transcript.push(BlockKind::Error, message),
                SessionAction::History => list_sessions(transcript, store),
                SessionAction::Doctor => {
                    transcript.push(BlockKind::System, crate::config::doctor::summary(runtime))
                }
                SessionAction::Resume(id) => resume(transcript, state, store, &id),
                SessionAction::Sql(sql) => {
                    result = Dispatch::SqlTask(super::sql_task::SqlTask {
                        profile: state.profile.clone(),
                        sql,
                        followup: super::sql_task::Followup::Sql {
                            connection: state.profile.clone(),
                        },
                    });
                }
                SessionAction::Contracts(command) => {
                    run_contracts(transcript, state, runtime, state_db, format, &command)
                }
                SessionAction::Export(path) => match last_query.as_ref() {
                    Some(lq) => {
                        result = Dispatch::SqlTask(super::sql_task::SqlTask {
                            profile: lq.connection.clone().or(state.profile.clone()),
                            sql: lq.sql.clone(),
                            followup: super::sql_task::Followup::Export { path },
                        });
                    }
                    None => transcript.push(
                        BlockKind::System,
                        "Nothing to export yet — run a query first.",
                    ),
                },
                SessionAction::Chart(args) => match last_query.as_ref() {
                    Some(lq) => {
                        let (kind, path) = parse_chart_args(&args);
                        result = Dispatch::SqlTask(super::sql_task::SqlTask {
                            profile: lq.connection.clone().or(state.profile.clone()),
                            sql: lq.sql.clone(),
                            followup: super::sql_task::Followup::Chart { kind, path },
                        });
                    }
                    None => transcript.push(
                        BlockKind::System,
                        "Nothing to chart yet — run a query first.",
                    ),
                },
                SessionAction::Explain(arg) => {
                    let (sql, connection) = if !arg.trim().is_empty() {
                        (arg.trim().to_string(), None)
                    } else if let Some(lq) = last_query.as_ref() {
                        (lq.sql.clone(), lq.connection.clone())
                    } else {
                        transcript.push(
                        BlockKind::System,
                        "Nothing to explain — run a query first, or pass SQL: /explain SELECT ...",
                    );
                        return result;
                    };
                    let trimmed = sql.trim().trim_end_matches(';').trim();
                    result = Dispatch::SqlTask(super::sql_task::SqlTask {
                        profile: connection.or_else(|| state.profile.clone()),
                        sql: format!("EXPLAIN {trimmed}"),
                        followup: super::sql_task::Followup::Explain,
                    });
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
        },
        Ok(None) => result = Dispatch::Agent(line.to_string()),
    }
    transcript.scroll_to_bottom();
    result
}

/// Parses "/chart [type] [path]": a leading known kind is consumed; anything
/// left is the output path.
fn parse_chart_args(args: &str) -> (Option<crate::chart::ChartKind>, Option<String>) {
    let mut tokens = args.split_whitespace();
    match tokens.next() {
        Some(tok) => match crate::chart::ChartKind::parse(tok) {
            Some(kind) => (Some(kind), tokens.next().map(str::to_string)),
            None => (None, Some(tok.to_string())),
        },
        None => (None, None),
    }
}
