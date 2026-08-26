//! Background SQL execution for direct commands (/sql, /export, /chart,
//! /explain). Running them on a worker thread keeps the UI responsive —
//! spinner, Esc, typing all stay live — instead of freezing the event loop
//! inside `block_on` for up to the query timeout.
//!
//! Dispatch captures everything the *completion* step needs (follow-up kind,
//! target path, connection) at command time; the loop polls the channel and
//! calls [`complete`] when the result lands.

use super::exec;
use super::transcript::{BlockKind, Transcript};
use crate::config::runtime::RuntimeConfig;
use crate::interactive::tui::types::LastQuery;
use crate::render::TerminalEvent;
use std::sync::Arc;
use std::sync::mpsc::Receiver;

/// What to do with a finished query result, captured when the command ran.
#[derive(Clone)]
pub(crate) enum Followup {
    /// Render the table and remember the query for /export & friends.
    Sql { connection: Option<String> },
    /// Write the result to a CSV/JSON path.
    Export { path: String },
    /// Build an HTML chart (optionally forced kind + output path).
    Chart {
        kind: Option<crate::chart::ChartKind>,
        path: Option<String>,
    },
    /// Render the plan table.
    Explain,
}

#[derive(Clone)]
pub(crate) struct SqlTask {
    pub profile: Option<String>,
    pub sql: String,
    pub followup: Followup,
}

/// Spawns the query on a worker thread with its own tokio runtime and returns
/// a non-blocking receiver for its [`TerminalEvent`].
pub(crate) fn spawn(runtime: Arc<RuntimeConfig>, task: SqlTask) -> Receiver<TerminalEvent> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let event = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime_handle) => {
                runtime_handle.block_on(exec::run_sql(&runtime, task.profile.as_deref(), &task.sql))
            }
            Err(error) => TerminalEvent::Error {
                message: error.to_string(),
            },
        };
        let _ = tx.send(event);
    });
    rx
}

/// Applies a finished result to the transcript according to its follow-up.
pub(crate) fn complete(
    task: &SqlTask,
    event: TerminalEvent,
    transcript: &mut Transcript,
    last_query: &mut Option<LastQuery>,
) {
    let TerminalEvent::QueryResult { result } = event else {
        let TerminalEvent::Error { message } = event else {
            return;
        };
        transcript.push(BlockKind::Error, message);
        return;
    };
    match &task.followup {
        Followup::Sql { connection } => {
            *last_query = Some(LastQuery {
                sql: task.sql.clone(),
                connection: connection.clone(),
            });
            transcript.push(BlockKind::Tool, super::table::format_table(&result));
        }
        Followup::Export { path } => {
            match super::export::write_result(&result, std::path::Path::new(path)) {
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
        Followup::Chart { kind, path } => {
            complete_chart(&result, *kind, path.as_deref(), transcript);
        }
        Followup::Explain => {
            transcript.push(BlockKind::Tool, super::table::format_plan(&result));
        }
    }
}

fn complete_chart(
    result: &saya_types::QueryResult,
    kind: Option<crate::chart::ChartKind>,
    path_arg: Option<&str>,
    transcript: &mut Transcript,
) {
    let mut spec = crate::chart::suggest_spec(result);
    if let Some(k) = kind {
        spec.kind = k;
    }
    let html = match crate::chart::render_html(result, &spec) {
        Ok(html) => html,
        Err(msg) => {
            transcript.push(BlockKind::System, msg);
            return;
        }
    };
    let path = path_arg
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("saya-chart.html"));
    if let Err(msg) = crate::chart::write_html(&html, &path) {
        transcript.push(BlockKind::Error, msg);
        return;
    }
    let mut note = format!("Chart written to {}", path.display());
    match crate::chart::open_file(&path) {
        Ok(()) => note.push_str(" (opening in your browser)"),
        Err(e) => note.push_str(&format!(" — open it manually ({e})")),
    }
    transcript.push(BlockKind::System, note);
}
