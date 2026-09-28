//! Query follow-ups: `/export`, `/chart`, and `/explain` re-run the last
//! query on a worker task with the matching follow-up. The export snapshot
//! is the exception: it writes the captured result itself, with no query.

use super::super::capture_agent::CaptureGap;
use super::super::transcript::{BlockKind, Transcript};
use super::super::types::LastQuery;
use super::chart::parse_chart_args;
use super::outcome::Dispatch;
use crate::interactive::session_commands::SessionAction;
use crate::interactive::session_state::SessionState;
use crate::slash::{ExportMode, ExportRequest};
use saya_types::{ExecutionEvidence, ResultScope};

pub(super) fn apply_query_actions(
    action: SessionAction,
    transcript: &mut Transcript,
    state: &mut SessionState,
    last_query: &mut Option<LastQuery>,
    captured: &Option<super::super::capture::CapturedResult>,
    capture_gap: Option<CaptureGap>,
) -> Option<Dispatch> {
    match action {
        SessionAction::Export(request) => apply_export(
            request,
            transcript,
            state,
            last_query,
            captured,
            capture_gap,
        ),
        SessionAction::Report(request) => {
            apply_report(&request, transcript, captured);
            None
        }
        SessionAction::Chart(args) => apply_chart(args, transcript, state, last_query),
        SessionAction::Explain(arg) => apply_explain(arg, transcript, state, last_query),
        _ => None,
    }
}

/// Writes the Markdown report from the captured result: no query, no task —
/// the report is the capture, plus provenance from its evidence. Nothing
/// here queries a database.
fn apply_report(
    request: &crate::slash::ReportRequest,
    transcript: &mut Transcript,
    captured: &Option<super::super::capture::CapturedResult>,
) {
    let Some(capture) = captured else {
        transcript.push(
            BlockKind::Error,
            "No captured result to report. Run the query with /sql first; captures \
             last only for this session.",
        );
        return;
    };
    let path = std::path::Path::new(&request.path);
    match super::super::export::write_report(
        &capture.result,
        &capture.evidence,
        request.rows,
        path,
        request.overwrite,
    ) {
        Ok(Some(n)) => transcript.push(
            BlockKind::System,
            format!(
                "Wrote report to {} ({n} rows included{})",
                request.path,
                scope_note(&capture.evidence)
            ),
        ),
        Ok(None) => transcript.push(
            BlockKind::System,
            format!(
                "Wrote report to {} (rows omitted{})",
                request.path,
                scope_note(&capture.evidence)
            ),
        ),
        Err(msg) => transcript.push(BlockKind::Error, msg),
    }
}

/// The scope label every report success message carries (D12): a
/// model-limited capture says what the rows are and that the full result may
/// be larger.
fn scope_note(evidence: &ExecutionEvidence) -> String {
    match &evidence.scope {
        ResultScope::Full => String::new(),
        ResultScope::ModelLimited { row_cap } => format!(
            "; model-limited: the agent saw only the first {row_cap} rows — \
             the full result may be larger"
        ),
    }
}

fn apply_export(
    request: ExportRequest,
    transcript: &mut Transcript,
    state: &mut SessionState,
    last_query: &mut Option<LastQuery>,
    captured: &Option<super::super::capture::CapturedResult>,
    capture_gap: Option<CaptureGap>,
) -> Option<Dispatch> {
    // The snapshot is the result the user already inspected: written here
    // from the capture, with no query dispatched at all.
    if request.mode == Some(ExportMode::Snapshot) {
        export_snapshot(&request, captured, capture_gap, transcript);
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
/// of. Nothing here queries a database. When the latest promoted query was
/// an agent query whose rows were not captured, the refusal names the gap —
/// the capture budget when the capture was refused, nothing when no capture
/// arrived — instead of the /sql-only wording.
fn export_snapshot(
    request: &ExportRequest,
    captured: &Option<super::super::capture::CapturedResult>,
    capture_gap: Option<CaptureGap>,
    transcript: &mut Transcript,
) {
    let Some(capture) = captured else {
        let message = match capture_gap {
            Some(gap) => format!(
                "The latest query's rows were not captured{}. \
                 Use /export --refresh to re-run it.",
                gap.reason()
            ),
            None => "No captured result to snapshot. Captures last only for this session and \
                 only for /sql results — run /sql again, or use /export --refresh."
                .to_string(),
        };
        transcript.push(BlockKind::Error, message);
        return;
    };
    let path = std::path::Path::new(&request.path);
    match super::super::export::write_result_overwrite(&capture.result, path, request.overwrite) {
        Ok(n) => {
            // D12: a model-limited capture is labelled as what it is — the
            // rows the agent saw, not the full result.
            let mut msg = if matches!(capture.evidence.scope, ResultScope::ModelLimited { .. }) {
                format!(
                    "Exported {n} row(s) the agent saw to {} from snapshot exec {} \
                         (model-limited: the full result may be larger — \
                         use --refresh for a full read)",
                    request.path,
                    capture.evidence.short_id(),
                )
            } else {
                format!(
                    "Exported {n} row(s) to {} from snapshot exec {} (captured {} UTC)",
                    request.path,
                    capture.evidence.short_id(),
                    super::super::capture::clock_hh_mm_ss(capture.evidence.finished_unix_ms),
                )
            };
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

#[cfg(test)]
#[path = "query_capture_tests.rs"]
mod capture_tests;

#[cfg(test)]
#[path = "query_tests.rs"]
mod tests;
