//! Plain session actions: history list, doctor summary, resume, direct SQL,
//! contracts, investigations, and run management. Owns its arms only: the
//! action is taken exactly when this helper owns the arm, and an owned arm
//! with no task to hand back still reports handled.

use super::super::dispatch_actions::{list_sessions, resume};
use super::super::dispatch_contracts::run_contracts;
use super::super::dispatch_investigation::run_investigation;
use super::super::dispatch_runs;
use super::super::transcript::{BlockKind, Transcript};
use super::super::types::LastQuery;
use super::outcome::Dispatch;
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session_commands::SessionAction;
use crate::interactive::session_runtime::SessionRuntime;
use crate::interactive::session_state::SessionState;
use crate::render::RenderFormat;
use saya_store::FsSessionStore;

#[allow(clippy::too_many_arguments)]
pub(super) fn apply_session_action(
    action: &mut Option<SessionAction>,
    transcript: &mut Transcript,
    state: &mut SessionState,
    runtime: &RuntimeConfig,
    store: &FsSessionStore,
    state_db: &saya_store::SqliteStateStore,
    format: RenderFormat,
    session: &mut SessionRuntime,
    last_query: &Option<LastQuery>,
) -> Option<Dispatch> {
    // The arms this helper owns. Anything else is left in `action`
    // untouched for the next helper in the chain.
    if !matches!(
        action,
        Some(
            SessionAction::History
                | SessionAction::Doctor
                | SessionAction::Resume(_)
                | SessionAction::Sql(_)
                | SessionAction::Contracts(_)
                | SessionAction::Investigation(_)
                | SessionAction::Runs(_)
                | SessionAction::RunCancel(_)
        )
    ) {
        return None;
    }
    let current = action.take()?;
    match current {
        SessionAction::History => list_sessions(transcript, store),
        SessionAction::Doctor => {
            transcript.push(BlockKind::System, crate::config::doctor::summary(runtime))
        }
        SessionAction::Resume(id) => resume(transcript, state, runtime, store, session, &id),
        SessionAction::Sql(sql) => {
            return Some(Dispatch::SqlTask(super::super::sql_task::SqlTask {
                profile: state.profile.clone(),
                sql,
                followup: super::super::sql_task::Followup::Sql {
                    connection: state.profile.clone(),
                },
                started_unix_ms: super::super::capture::unix_now_ms(),
            }));
        }
        SessionAction::Contracts(command) => {
            run_contracts(transcript, state, runtime, state_db, format, &command)
        }
        SessionAction::Investigation(command) => {
            // The saved-investigation adapter: the shared `run_investigation`
            // dispatcher plus the TUI's save-without-SQL fill (the last
            // selectable query, read-only here — the fill never edits it).
            // `run` returns a replay task for the caller; every other
            // subcommand ran inline (its block is already pushed), so the
            // chain hears "handled", never "not mine".
            return Some(
                run_investigation(transcript, runtime, state_db, format, &command, last_query)
                    .unwrap_or(Dispatch::Handled),
            );
        }
        SessionAction::Runs(run_id) => {
            let command = match run_id {
                Some(run_id) => crate::cli::RunCommand::Show { run_id },
                None => crate::cli::RunCommand::List,
            };
            dispatch_runs::run_management_command(transcript, runtime, state_db, format, command);
        }
        SessionAction::RunCancel(run_id) => dispatch_runs::run_management_command(
            transcript,
            runtime,
            state_db,
            format,
            crate::cli::RunCommand::Cancel { run_id },
        ),
        // The guard above decided ownership; if it ever drifts, put the
        // action back and report "not mine" rather than dropping or panicking.
        other => {
            *action = Some(other);
            return None;
        }
    }
    Some(Dispatch::Handled)
}
