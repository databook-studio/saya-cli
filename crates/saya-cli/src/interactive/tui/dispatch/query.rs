//! Query follow-ups: `/export`, `/chart`, and `/explain` re-run the last
//! query on a worker task with the matching follow-up.

use super::super::transcript::{BlockKind, Transcript};
use super::super::types::LastQuery;
use super::chart::parse_chart_args;
use super::outcome::Dispatch;
use crate::interactive::session_commands::SessionAction;
use crate::interactive::session_state::SessionState;

pub(super) fn apply_query_actions(
    action: SessionAction,
    transcript: &mut Transcript,
    state: &mut SessionState,
    last_query: &mut Option<LastQuery>,
) -> Option<Dispatch> {
    match action {
        SessionAction::Export(path) => apply_export(path, transcript, state, last_query),
        SessionAction::Chart(args) => apply_chart(args, transcript, state, last_query),
        SessionAction::Explain(arg) => apply_explain(arg, transcript, state, last_query),
        _ => None,
    }
}

fn apply_export(
    path: String,
    transcript: &mut Transcript,
    state: &mut SessionState,
    last_query: &mut Option<LastQuery>,
) -> Option<Dispatch> {
    match last_query.as_ref() {
        Some(lq) => {
            return Some(Dispatch::SqlTask(super::super::sql_task::SqlTask {
                profile: lq.connection.clone().or(state.profile.clone()),
                sql: lq.sql.clone(),
                followup: super::super::sql_task::Followup::Export { path },
            }));
        }
        None => transcript.push(
            BlockKind::System,
            "Nothing to export yet — run a query first.",
        ),
    }
    None
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
    }))
}
