//! Plain session actions: history list, doctor summary, resume, direct SQL,
//! contracts, investigations, and run management. Owns its arms only: the
//! action is taken exactly when this helper owns the arm, and an owned arm
//! with no task to hand back still reports handled.

use super::super::dispatch_actions::{list_sessions, resume, rotate};
use super::super::dispatch_contracts::run_contracts;
use super::super::dispatch_investigation::run_investigation;
use super::super::dispatch_runs;
use super::super::transcript::{BlockKind, Transcript};
use super::super::types::LastQuery;
use super::outcome::Dispatch;
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session::commands::SessionAction;
use crate::interactive::session::runtime::SessionRuntime;
use crate::interactive::session::state::SessionState;
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
                | SessionAction::Context
                | SessionAction::SessionsList
                | SessionAction::SessionsDelete(_)
                | SessionAction::Preferences(_)
                | SessionAction::Recovery(_)
                | SessionAction::Resume(_)
                | SessionAction::Rotate
                | SessionAction::Sql(_)
                | SessionAction::Contracts(_)
                | SessionAction::Investigation(_)
                | SessionAction::Runs(_)
                | SessionAction::RunCancel(_)
                | SessionAction::Setup
                | SessionAction::Reload
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
        SessionAction::Context => {
            // `/context` shows the instruction sources the next request
            // would load through the one shared renderer the headless loop
            // prints too — names, sizes, short digests, reasons; never the
            // file text.
            transcript.push(
                BlockKind::System,
                crate::interactive::session::context_view::context_view(runtime, session.root()),
            );
        }
        SessionAction::Resume(id) => resume(transcript, state, runtime, store, session, &id),
        SessionAction::Rotate => rotate(transcript, state, runtime, store, session),
        SessionAction::SessionsList => {
            // `/sessions list` — the same listing the headless REPL prints,
            // through the one shared builder: rows, pending deletions, and
            // the storage root. The root is the one the store was built
            // from (`default_session_dir`); a store failure is an Error
            // block, never a silent empty listing.
            match crate::interactive::session::commands::sessions_list_text(
                store,
                &crate::interactive::session::lifecycle::paths::default_session_dir(),
            ) {
                Ok(text) => transcript.push(BlockKind::System, text),
                Err(text) => transcript.push(BlockKind::Error, text),
            }
        }
        SessionAction::SessionsDelete(id) => {
            // `/sessions delete <id>` — the shared delete the headless
            // `saya sessions delete` runs, never prompting. This session's
            // own id is passed as the current one, so the TUI can never
            // delete itself underneath its owner; every refusal lands as
            // an Error block naming what was not deleted.
            let (ok, message) = crate::interactive::session::commands::sessions_delete_text(
                store,
                &crate::interactive::session::lifecycle::paths::default_session_dir(),
                &id,
                Some(&state.id),
            );
            transcript.push(
                if ok {
                    BlockKind::System
                } else {
                    BlockKind::Error
                },
                message,
            );
        }
        SessionAction::Preferences(command) => {
            // `/preferences …` — the shared project-preference operations
            // the headless `saya preferences` runs, scoped to the session's
            // bound worktree root (never the launch cwd; no bound root is
            // the shared NoWorktree refusal). The words are the headless
            // rendering; a refusal (exit 2) is an Error block, never a
            // silent no-op.
            let (code, message) = crate::interactive::session::lifecycle::resume::block_on(
                crate::commands::preferences::run_in_session(command, session.root(), state_db),
            );
            transcript.push(
                if code == 0 {
                    BlockKind::System
                } else {
                    BlockKind::Error
                },
                message,
            );
        }
        SessionAction::Recovery(command) => {
            // `/recovery …` — the one shared `run_recovery` operation the
            // headless loop runs too, over the session's own recovery
            // handle (its store, its id, its flag — the recorder's parts)
            // and the bound workspace. The words are the shared wording; a
            // refusal is an Error block, never a silent no-op. The restore
            // itself cannot overlap a running request: a line typed while
            // one runs is queued and dispatched only once it finishes (the
            // same gate the typed `/resume` and `/compact` pass through —
            // `loop_tick::pending`), so a turn is never interrupted
            // mid-write.
            let (ok, message) = crate::commands::recovery::run_recovery(
                command,
                session.recovery(),
                session.universe().workspace().as_deref(),
                crate::interactive::session::continuation::now_unix_ms(),
            );
            transcript.push(
                if ok {
                    BlockKind::System
                } else {
                    BlockKind::Error
                },
                message,
            );
        }
        SessionAction::Sql(sql) => {
            return Some(Dispatch::SqlTask(super::super::sql_task::SqlTask::new(
                state.profile.clone(),
                sql,
                super::super::sql_task::Followup::Sql {
                    connection: state.profile.clone(),
                },
            )));
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
        // `/setup` routes the handoff: the loop's tick_pending owns the
        // terminal suspension and the wizard run under the busy gate — the
        // dispatch only picks the outcome, it never runs the wizard.
        SessionAction::Setup => return Some(Dispatch::RunSetup),
        // `/reload` routes the swap the same way: tick_pending owns it (it
        // holds the app's runtime handle and the launch options); the
        // dispatch only picks the outcome.
        SessionAction::Reload => return Some(Dispatch::Reload),
        // The guard above decided ownership; if it ever drifts, put the
        // action back and report "not mine" rather than dropping or panicking.
        other => {
            *action = Some(other);
            return None;
        }
    }
    Some(Dispatch::Handled)
}
