//! Query follow-ups: `/export`, `/chart`, and `/explain` re-run the last
//! query on a worker task with the matching follow-up. The export snapshot
//! is the exception: it writes the captured result itself, with no query.

use super::super::transcript::{BlockKind, Transcript};
use super::super::types::LastQuery;
use super::chart::parse_chart_args;
use super::outcome::Dispatch;
use crate::interactive::session_commands::SessionAction;
use crate::interactive::session_state::SessionState;
use crate::slash::{ExportMode, ExportRequest};

pub(super) fn apply_query_actions(
    action: SessionAction,
    transcript: &mut Transcript,
    state: &mut SessionState,
    last_query: &mut Option<LastQuery>,
    captured: &Option<super::super::capture::CapturedResult>,
) -> Option<Dispatch> {
    match action {
        SessionAction::Export(request) => {
            apply_export(request, transcript, state, last_query, captured)
        }
        SessionAction::Chart(args) => apply_chart(args, transcript, state, last_query),
        SessionAction::Explain(arg) => apply_explain(arg, transcript, state, last_query),
        _ => None,
    }
}

fn apply_export(
    request: ExportRequest,
    transcript: &mut Transcript,
    state: &mut SessionState,
    last_query: &mut Option<LastQuery>,
    captured: &Option<super::super::capture::CapturedResult>,
) -> Option<Dispatch> {
    // The snapshot is the result the user already inspected: written here
    // from the capture, with no query dispatched at all.
    if request.mode == Some(ExportMode::Snapshot) {
        export_snapshot(&request, captured, transcript);
        return None;
    }
    match last_query.as_ref() {
        Some(lq) => {
            return Some(Dispatch::SqlTask(super::super::sql_task::SqlTask {
                profile: lq.connection.clone().or(state.profile.clone()),
                sql: lq.sql.clone(),
                followup: super::super::sql_task::Followup::Export { request },
                started_unix_ms: super::super::capture::unix_now_ms(),
            }));
        }
        None => transcript.push(
            BlockKind::System,
            "Nothing to export yet — run a query first.",
        ),
    }
    None
}

/// Writes the captured result to the destination, naming the execution and
/// the capture time so the user can tell exactly which run a snapshot is
/// of. Nothing here queries a database.
fn export_snapshot(
    request: &ExportRequest,
    captured: &Option<super::super::capture::CapturedResult>,
    transcript: &mut Transcript,
) {
    let Some(capture) = captured else {
        transcript.push(
            BlockKind::Error,
            "No captured result to snapshot. Captures last only for this session and \
             only for /sql results — run /sql again, or use /export --refresh.",
        );
        return;
    };
    let path = std::path::Path::new(&request.path);
    match super::super::export::write_result_overwrite(&capture.result, path, request.overwrite) {
        Ok(n) => {
            let mut msg = format!(
                "Exported {n} row(s) to {} from snapshot exec {} (captured {} UTC)",
                request.path,
                capture.evidence.short_id(),
                super::super::capture::clock_hh_mm_ss(capture.evidence.finished_unix_ms),
            );
            if capture.result.truncated {
                msg.push_str(" (result was truncated)");
            }
            transcript.push(BlockKind::System, msg);
        }
        Err(msg) => transcript.push(BlockKind::Error, msg),
    }
}

fn apply_chart(
    args: String,
    transcript: &mut Transcript,
    state: &mut SessionState,
    last_query: &mut Option<LastQuery>,
) -> Option<Dispatch> {
    match last_query.as_ref() {
        Some(lq) => {
            let (kind, path) = parse_chart_args(&args);
            return Some(Dispatch::SqlTask(super::super::sql_task::SqlTask {
                profile: lq.connection.clone().or(state.profile.clone()),
                sql: lq.sql.clone(),
                followup: super::super::sql_task::Followup::Chart { kind, path },
                started_unix_ms: super::super::capture::unix_now_ms(),
            }));
        }
        None => transcript.push(
            BlockKind::System,
            "Nothing to chart yet — run a query first.",
        ),
    }
    None
}

fn apply_explain(
    arg: String,
    transcript: &mut Transcript,
    state: &mut SessionState,
    last_query: &mut Option<LastQuery>,
) -> Option<Dispatch> {
    let (sql, connection) = if !arg.trim().is_empty() {
        (arg.trim().to_string(), None)
    } else if let Some(lq) = last_query.as_ref() {
        (lq.sql.clone(), lq.connection.clone())
    } else {
        transcript.push(
            BlockKind::System,
            "Nothing to explain — run a query first, or pass SQL: /explain SELECT ...",
        );
        return None;
    };
    let trimmed = sql.trim().trim_end_matches(';').trim();
    Some(Dispatch::SqlTask(super::super::sql_task::SqlTask {
        profile: connection.or_else(|| state.profile.clone()),
        sql: format!("EXPLAIN {trimmed}"),
        followup: super::super::sql_task::Followup::Explain,
        started_unix_ms: super::super::capture::unix_now_ms(),
    }))
}
