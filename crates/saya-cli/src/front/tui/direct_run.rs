//! What a direct command does off the loop's thread (`03` §4.1).
//!
//! Every one of them is today's code, called rather than re-written:
//! `/sql` runs through [`sql_task`](crate::interactive::tui::SqlTask)'s worker —
//! the bounded, read-only path through the safety layer, with its permit and
//! its executed-source envelope — and reports the rows it handed back; `/export`
//! writes through today's two-phase export; `/chart` renders and writes through
//! today's chart; `/investigation` parses with the crate's one parser, fills a
//! bare `save` from the last query the surface shows (today's fill) and runs
//! through the shared dispatcher.
//!
//! The point of the module is where it runs: a query takes as long as it takes,
//! so the runtime performs a command on a blocking task and hands the answer
//! back as [`Msg::Worker`](crate::front::tui::Msg::Worker).

use std::sync::Arc;

use saya_agent::CancellationToken;
use saya_store::SqliteStateStore;
use saya_types::QueryResult;

use super::direct::{Direct, DirectOutcome};
use super::direct_trust::TrustStore;
use super::msg::Worker;
use crate::config::runtime::RuntimeConfig;
use crate::front::ResultPreview;
use crate::interactive::tui::{
    CAP_REFUSAL, Followup, LastQuery, SqlTask, fill_save_from_last_query, parse_chart_args,
    spawn_sql_task, try_acquire_worker_permit, write_result_for,
};
use crate::render::{RenderFormat, TerminalEvent};

/// What the surface says when a command's worker went away without a word.
pub(crate) const WORKER_GONE: &str = "the command's worker ended without reporting.";

/// What a direct command needs from the launch: the composed runtime its query
/// runs against, the state store `/investigation` writes through, the render
/// format, and the store a workspace answer binds through.
pub(crate) struct DirectDeps {
    pub(crate) runtime: Arc<RuntimeConfig>,
    pub(crate) state_db: SqliteStateStore,
    pub(crate) format: RenderFormat,
    pub(crate) trust: Arc<dyn TrustStore>,
}

/// Performs one direct command and answers what the surface is handed back.
pub(crate) fn perform(deps: &DirectDeps, command: Direct) -> Worker {
    match command {
        Direct::Sql { profile, sql } => query(deps, profile, sql),
        Direct::Export {
            path,
            overwrite,
            result,
        } => export(&result, &path, overwrite),
        Direct::Chart { args, result } => chart(&result, &args),
        Direct::Investigation {
            command,
            last_query,
        } => investigation(deps, command, last_query),
    }
}

/// `/export`: today's two-phase write, to the path the operator named.
fn export(result: &QueryResult, path: &str, overwrite: bool) -> Worker {
    // The one export request the slash grammar builds: the destination's
    // extension selects the format, as K7's `/export` does.
    let request = crate::slash::ExportRequest {
        overwrite,
        path: path.to_owned(),
        ..crate::slash::ExportRequest::default()
    };
    match write_result_for(result, &request) {
        Ok(rows) => Worker {
            message: format!("Exported {rows} row(s) to {path}"),
            ..Worker::default()
        },
        Err(message) => Worker {
            direct: Some(DirectOutcome::Error(message)),
            ..Worker::default()
        },
    }
}

/// `/chart`: today's suggester and renderer, written where the operator asked
/// — a reserved temp file otherwise — and said in one toast.
fn chart(result: &QueryResult, args: &str) -> Worker {
    let (kind, path_arg) = parse_chart_args(args);
    let mut spec = crate::chart::suggest_spec(result);
    if let Some(kind) = kind {
        spec.kind = kind;
    }
    let html = match crate::chart::render_html(result, &spec) {
        Ok(html) => html,
        Err(message) => {
            return Worker {
                message,
                ..Worker::default()
            };
        }
    };
    let explicit = path_arg.is_some();
    let path = match path_arg {
        Some(path) => std::path::PathBuf::from(path),
        None => match crate::chart::reserve_temp_chart() {
            Ok(mut chart) => {
                if let Err(message) = chart.write_html(&html) {
                    return Worker {
                        direct: Some(DirectOutcome::Error(message)),
                        ..Worker::default()
                    };
                }
                chart.path().to_path_buf()
            }
            Err(message) => {
                return Worker {
                    direct: Some(DirectOutcome::Error(message)),
                    ..Worker::default()
                };
            }
        },
    };
    if explicit && let Err(message) = crate::chart::write_html(&html, &path) {
        return Worker {
            direct: Some(DirectOutcome::Error(message)),
            ..Worker::default()
        };
    }
    let mut note = format!("Chart written to {}", path.display());
    if result.truncated {
        note.push_str(" (result was truncated)");
    }
    match crate::chart::open_file(&path) {
        Ok(()) => note.push_str(" (opening in your browser)"),
        Err(error) => note.push_str(&format!(" — open it manually ({error})")),
    }
    Worker {
        message: note,
        ..Worker::default()
    }
}

/// `/investigation`: the crate's own command, today's save fill, and the shared
/// dispatcher's captured output.
fn investigation(
    deps: &DirectDeps,
    command: crate::cli::InvestigationCommand,
    last_query: Option<LastQuery>,
) -> Worker {
    let command = match fill_save_from_last_query(command, &last_query) {
        Ok(command) => command,
        Err(message) => {
            return Worker {
                message,
                ..Worker::default()
            };
        }
    };
    crate::commands::capture_output_start();
    let outcome = crate::interactive::session::lifecycle::resume::block_on(
        crate::commands::run_investigation(
            command,
            &deps.runtime,
            deps.format,
            // The surface never reads stdin: a missing secret is an error
            // rather than a prompt inside the alternate screen.
            false,
            &deps.state_db,
        ),
    );
    let (out, err) = crate::commands::capture_output_take();
    let body = if out.trim().is_empty() { err } else { out };
    let body = body.trim_end().to_owned();
    match outcome {
        Ok(0) => Worker {
            message: body,
            ..Worker::default()
        },
        Ok(_) => Worker {
            direct: Some(DirectOutcome::Error(body)),
            ..Worker::default()
        },
        Err(error) => Worker {
            direct: Some(DirectOutcome::Error(error.to_string())),
            ..Worker::default()
        },
    }
}

/// `/sql`: today's worker task, waited on. The permit is acquired before the
/// spawn, as today's admission does, so the process-wide cap holds a direct
/// query from the surface exactly as it holds one from the old TUI.
fn query(deps: &DirectDeps, profile: Option<String>, sql: String) -> Worker {
    let Some(permit) = try_acquire_worker_permit() else {
        return Worker {
            message: CAP_REFUSAL.to_owned(),
            ..Worker::default()
        };
    };
    let connection = profile
        .clone()
        .or_else(|| deps.runtime.resolved.profile_name.clone());
    let task = SqlTask::new(
        profile,
        sql.clone(),
        Followup::Sql {
            connection: connection.clone(),
        },
    );
    let events = spawn_sql_task(
        permit,
        Arc::clone(&deps.runtime),
        task,
        CancellationToken::new(),
    );
    match events.recv() {
        Ok(TerminalEvent::QueryResult { result }) => Worker {
            direct: Some(DirectOutcome::Result(ResultPreview {
                connection: connection.unwrap_or_default(),
                sql,
                result,
                refusal: None,
            })),
            ..Worker::default()
        },
        // A statement the read-only policy refused arrives here, in the safety
        // layer's own words, and is filed as the refusal line — never as rows.
        Ok(TerminalEvent::Error { message }) => Worker {
            direct: Some(DirectOutcome::Error(message)),
            ..Worker::default()
        },
        Ok(_) => Worker {
            message: WORKER_GONE.to_owned(),
            ..Worker::default()
        },
        Err(_) => Worker {
            message: WORKER_GONE.to_owned(),
            ..Worker::default()
        },
    }
}
