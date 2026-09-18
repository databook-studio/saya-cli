//! Routes a submitted input line to command handling and renders the outcome
//! into the transcript. Slash commands reuse the shared `state.apply` logic;
//! results that would normally print to stdout are captured via `render_event`
//! and pushed into the transcript instead.

use super::super::session_runtime::SessionRuntime;
use super::dispatch_actions::{list_sessions, resume};
use super::dispatch_contracts::run_contracts;
use super::dispatch_runs;
use super::transcript::{BlockKind, Transcript};
use super::types::LastQuery;
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session_commands::SessionAction;
use crate::interactive::session_run::{self, RunTail};
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
    /// `/compact` — shrink working memory through a bounded summariser call.
    /// Runs on a worker task like a SQL command; the caller owns the handle.
    Compact,
    /// `/run <goal…>` — a fresh run the run panel drives as a worker task;
    /// the caller owns the panel state and the worker handle.
    RunPanel {
        goal: Option<String>,
        allow: Vec<String>,
        budget: Vec<String>,
    },
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
    session: &mut SessionRuntime,
) -> Dispatch {
    // In the TUI, /sessions opens an interactive picker rather than a text list.
    if line.trim() == "/sessions" {
        return Dispatch::OpenSessionPicker;
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
                SessionAction::History => list_sessions(transcript, store),
                SessionAction::Doctor => {
                    transcript.push(BlockKind::System, crate::config::doctor::summary(runtime))
                }
                SessionAction::Resume(id) => {
                    resume(transcript, state, runtime, store, session, &id)
                }
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
                SessionAction::Runs(run_id) => {
                    let command = match run_id {
                        Some(run_id) => crate::cli::RunCommand::Show { run_id },
                        None => crate::cli::RunCommand::List,
                    };
                    dispatch_runs::run_management_command(
                        transcript, runtime, state_db, format, command,
                    );
                }
                SessionAction::RunCancel(run_id) => dispatch_runs::run_management_command(
                    transcript,
                    runtime,
                    state_db,
                    format,
                    crate::cli::RunCommand::Cancel { run_id },
                ),
                SessionAction::Allow(tokens) => {
                    // `/allow <scopes…>` seeds the session's one grant store
                    // through the shared behaviour — the same parser, the
                    // session surface, and the same composition the prompts
                    // state: a token the session composed no capability for
                    // is refused there too, never seeded. A refused scope is
                    // an error and seeds nothing; `/allow none` seeds
                    // nothing and says so. Each newly seeded token is
                    // journalled once by the shared behaviour; a failed
                    // journal write changes no grant and is said in the
                    // message.
                    match crate::interactive::session_grants::allow(
                        &tokens,
                        &session.universe().approval_facts(runtime),
                        session.policy().grants(),
                        &session.journal(),
                    ) {
                        Ok(message) => transcript.push(BlockKind::System, message),
                        Err(error) => transcript.push(BlockKind::Error, error),
                    }
                }
                SessionAction::Grants => {
                    // `/grants` lists the store verbatim: the words are the
                    // record, the same words the prompts offered. The mode
                    // is the engine's own — under bypass it is stated first,
                    // so the count never reads "nothing runs".
                    transcript.push(
                        BlockKind::System,
                        crate::interactive::session_grants::listing(
                            session.policy().mode(),
                            session.policy().grants(),
                        ),
                    );
                }
                SessionAction::Run(tail) => {
                    // The child's own grammar parses the tail in-process (the
                    // parser stays the authority); a fresh run drives the
                    // run panel as a worker task instead of the nested child
                    // a piped session spawns — the alternate screen owns
                    // stdout, so the run's observers forward to the panel.
                    // `--seed-grants` is the adapter's word: the session's
                    // grants are filtered through the run's parser first —
                    // accepted ones join the child's `--allow`, refused ones
                    // are named before the panel opens.
                    let (seed_requested, tail) = session_run::separate_seed_flag(&tail);
                    let seed = if seed_requested {
                        let seed = crate::interactive::session_grants::run_seed(
                            &session.policy().grants().tokens(),
                        );
                        transcript.push(
                            BlockKind::System,
                            crate::interactive::session_grants::seed_message(&seed),
                        );
                        Some(seed)
                    } else {
                        None
                    };
                    match session_run::parse_run_tail(tail) {
                        Ok(RunTail::Start {
                            goal,
                            mut allow,
                            budget,
                        }) => {
                            if let Some(seed) = seed.as_ref() {
                                allow.extend(seed.forwarded.iter().cloned());
                            }
                            result = Dispatch::RunPanel {
                                goal,
                                allow,
                                budget,
                            };
                        }
                        Ok(RunTail::Manage(command)) => {
                            dispatch_runs::run_management_command(
                                transcript, runtime, state_db, format, command,
                            );
                        }
                        Ok(RunTail::Resume(run_id)) => transcript.push(
                            BlockKind::System,
                            format!(
                                "Resuming run {run_id} streams to the real terminal, which \
                                 the TUI does not own while the panel is up — resume it \
                                 from a shell: `saya run resume {run_id}`."
                            ),
                        ),
                        Err(message) => transcript.push(BlockKind::Error, message),
                    }
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
                SessionAction::Compact => {
                    result = Dispatch::Compact;
                }
                SessionAction::Agent(_) | SessionAction::Cancelled => {}
                SessionAction::NotImplemented(feature) => {
                    transcript.push(BlockKind::System, format!("Not implemented: {feature}"))
                }
                SessionAction::Exit => result = Dispatch::Quit,
            },
        },
        Ok(None) => result = Dispatch::Agent(line.to_string()),
    }
    if approvals_set {
        if let Some(activation) = crate::interactive::session_activation::line_if_bypass(
            state,
            runtime,
            &session.universe(),
        ) {
            transcript.push(BlockKind::System, activation);
        }
        if crate::interactive::session_activation::bypass_activated_by_command(
            &before_mode,
            &state.approval_mode,
        ) && let Err(error) = session
            .journal()
            .bypass_activated(saya_store::BypassSource::Command)
        {
            transcript.push(
                BlockKind::Error,
                crate::interactive::session_grants::journal_warning(&error),
            );
        }
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
