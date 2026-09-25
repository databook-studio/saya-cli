//! Plain session actions: history list, doctor summary, resume, direct SQL,
//! contracts, and run management. Returns a worker task for the caller when
//! the action needs one; anything else renders inline and returns `None`.

use super::super::dispatch_actions::{list_sessions, resume};
use super::super::dispatch_contracts::run_contracts;
use super::super::dispatch_runs;
use super::super::transcript::{BlockKind, Transcript};
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
) -> Option<Dispatch> {
    match action.take().expect("dispatch passes the action through") {
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
            }));
        }
        SessionAction::Contracts(command) => {
            run_contracts(transcript, state, runtime, state_db, format, &command)
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
        other => {
            *action = Some(other);
            return None;
        }
    }
    None
}
