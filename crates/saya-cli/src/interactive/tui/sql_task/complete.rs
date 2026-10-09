//! Applying a finished SQL result to the transcript according to the
//! task's follow-up: the table render with its execution-evidence line,
//! the export, the chart, and the plan.

use super::super::capture;
use super::super::export;
use super::super::table;
use super::super::transcript::{BlockKind, Transcript};
use super::task::{Followup, SqlTask};
use crate::interactive::tui::types::LastQuery;
use crate::render::TerminalEvent;

/// Applies a finished result to the transcript according to its follow-up.
/// The `Followup::Sql` success path also updates the app's ephemeral capture
/// (`captured`) and stamps the execution evidence line onto the table block;
/// every other path — errors, `/export`, `/chart`, `/explain` — leaves the
/// capture exactly as it was.
pub(crate) fn complete(
    task: &SqlTask,
    event: TerminalEvent,
    transcript: &mut Transcript,
    last_query: &mut Option<LastQuery>,
    captured: &mut Option<capture::CapturedResult>,
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
            let table = table::with_scope_line(
                table::format_table(&result),
                connection.as_deref(),
                &result.executed_sql,
            );
            let Some(execution) = task.take_execution() else {
                // No envelope: the worker never ran (or failed before it
                // could write one). Render the result with no evidence and
                // capture nothing — never synthesize provenance from the
                // live runtime.
                transcript.push(BlockKind::Table, table);
                return;
            };
            let evidence =
                capture::direct_sql_evidence_from_execution(connection.as_deref(), &execution);
            // One final line under the scope line: the execution's
            // provenance, no SQL text in it.
            transcript.push(
                BlockKind::Table,
                format!("{table}\n{}", evidence.human_line()),
            );
            let accounted = capture::accounted_bytes(&result);
            capture::capture_within(captured, result, evidence, accounted, transcript);
        }
        Followup::Export { request } => {
            match export::write_result_overwrite(
                &result,
                std::path::Path::new(&request.path),
                request.overwrite,
            ) {
                Ok(n) => {
                    let mut msg = format!(
                        "Exported {n} row(s) to {} — refreshed: re-ran the query just now",
                        request.path
                    );
                    if result.truncated {
                        msg.push_str(" (result was truncated)");
                    }
                    // Only the legacy form points at --snapshot, and only
                    // when there is a capture to point at: the result the
                    // user inspected is exactly what --snapshot would write.
                    if request.mode.is_none() && captured.is_some() {
                        msg.push_str(" (use --snapshot to export the result you already have)");
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
            transcript.push(BlockKind::Tool, table::format_plan(&result));
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
    let path = match path_arg {
        Some(path) => std::path::PathBuf::from(path),
        None => match crate::chart::reserve_temp_chart() {
            Ok(mut chart) => {
                if let Err(msg) = chart.write_html(&html) {
                    transcript.push(BlockKind::Error, msg);
                    return;
                }
                chart.path().to_path_buf()
            }
            Err(msg) => {
                transcript.push(BlockKind::Error, msg);
                return;
            }
        },
    };
    if path_arg.is_some()
        && let Err(msg) = crate::chart::write_html(&html, &path)
    {
        transcript.push(BlockKind::Error, msg);
        return;
    }
    let mut note = format!("Chart written to {}", path.display());
    if result.truncated {
        note.push_str(" (result was truncated)");
    }
    match crate::chart::open_file(&path) {
        Ok(()) => note.push_str(" (opening in your browser)"),
        Err(e) => note.push_str(&format!(" — open it manually ({e})")),
    }
    transcript.push(BlockKind::System, note);
}
