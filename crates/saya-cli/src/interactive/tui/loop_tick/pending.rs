//! Draining one queued prompt once the request ends.

use super::super::application::SecondSqlDecision;
use super::super::dispatch::Dispatch;
use super::super::session_save::queue_session_save;
use super::super::sql_task;
use super::super::transcript::BlockKind;
use super::super::types::App;
use crate::config::runtime::RuntimeConfig;
use crate::interactive::compact_task;
use crate::interactive::session_runtime::SessionRuntime;
use crate::interactive::session_state::SessionState;
use crate::render::RenderFormat;
use saya_store::FsSessionStore;
use std::sync::Arc;

// Queued prompts (submitted while busy) wait until the request ends.
pub(crate) fn tick_pending(
    app: &mut App,
    state: &mut SessionState,
    runtime: &RuntimeConfig,
    store: &FsSessionStore,
    format: RenderFormat,
    session: &mut SessionRuntime,
) {
    if !app.is_busy()
        && let Some(line) = app.pending.take()
    {
        let id_before = state.id.clone();
        let outcome = super::super::dispatch::dispatch(
            &line,
            &mut app.transcript,
            &app.profiles,
            state,
            runtime,
            store,
            &app.state_db,
            format,
            &mut app.last_query,
            session,
        );
        match outcome {
            Dispatch::Quit => {
                // An in-flight run is not orphaned by a quit: the quit is
                // refused until the run is cancelled or finished.
                if app.try_quit() {
                    app.should_quit = true;
                }
            }
            // A command may have switched profiles; refresh @-references.
            Dispatch::Handled => app.reload_at_refs(state),
            Dispatch::Agent(prompt) => {
                app.start_agent(prompt, state, session);
            }
            Dispatch::OpenSessionPicker => app.open_session_picker(store),
            Dispatch::Compact => {
                compact_task::start(&mut *app, state);
            }
            Dispatch::SetColumns(arg) => app.set_visible_columns(arg),
            Dispatch::RunPanel {
                goal,
                allow,
                budget,
            } => app.start_run_panel(goal, allow, budget, format, state),
            Dispatch::SqlTask(task) => {
                // One SQL command in flight at a time. The queued-prompt
                // gate (`!is_busy()`, which now covers SQL tasks) is the
                // primary defence: a second command submitted while one
                // runs is held until the first finishes. This guard is the
                // backstop — should a SqlTask reach the handler while one
                // is already running, refuse rather than silently drop the
                // first result.
                match app.admit_second_sql() {
                    SecondSqlDecision::Start => {
                        let started = std::time::Instant::now();
                        // Share the existing `Arc<RuntimeConfig>` instead of
                        // deep-cloning the whole config (resolved plaintext
                        // secrets included) onto a detached thread per
                        // command.
                        app.sql_task = Some((
                            sql_task::spawn(Arc::clone(&app.runtime), task.clone()),
                            task,
                            started,
                        ));
                        // Reuse the agent status fields so the status bar
                        // (which reads them) shows "running query Ns" with
                        // a spinner while the query runs. A
                        // SQL task and an agent stream never run
                        // concurrently — the gate prevents dispatch while
                        // either is busy — so these fields are free to reuse.
                        app.request.started = Some(started);
                        app.request.activity = Some("query".into());
                    }
                    SecondSqlDecision::Reject(message) => {
                        app.transcript.push(BlockKind::System, message)
                    }
                }
            }
        }
        // A `/resume` swapped the session: the engine side too, so the
        // app's universe is the resumed session's, not the old one's.
        // The live list is seeded with the swap, so the next turn sees
        // what the resumed record carried.
        if state.id != id_before {
            app.session = session.universe();
            app.session.seed_tasks(state.task_list.clone());
        }
        queue_session_save(&mut *app, store, state);
    }
}
