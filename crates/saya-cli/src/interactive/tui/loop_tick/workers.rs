//! Non-blocking worker polls: the run panel, `/compact`, and direct SQL.

use super::super::session_save::{poll_session_save, queue_session_save};
use super::super::sql_task;
use super::super::transcript::BlockKind;
use super::super::types::App;
use crate::interactive::compact_task;
use crate::interactive::session_state::SessionState;
use saya_store::FsSessionStore;

// Poll the run worker (non-blocking): the panel's step list, its
// lifecycle line, and its episode transcript advance when the run
// has news; the event loop never blocks on the run.
pub(crate) fn tick_workers(app: &mut App, store: &FsSessionStore, state: &mut SessionState) {
    poll_session_save(app, store);

    app.poll_run_panel(state.show_thinking);

    // Poll the /compact worker (non-blocking): apply its result when ready.
    if app.compact_task.is_some() {
        compact_task::poll(&mut *app, state);
        queue_session_save(&mut *app, store, state);
    }

    // Poll the direct-SQL worker (non-blocking): apply its result when ready.
    if let Some((rx, task, _started)) = app.sql_task.as_ref() {
        match rx.try_recv() {
            Ok(event) => {
                let task = task.clone();
                app.sql_task = None;
                // The query is done; drop the status fields the bar reused
                // for it (no agent stream is concurrent, so they are ours).
                app.request.started = None;
                app.request.activity = None;
                sql_task::complete(&task, event, &mut app.transcript, &mut app.last_query);
                // A new result table starts at its first column so the
                // view does not inherit a scroll position from an earlier,
                // differently-shaped table.
                if matches!(task.followup, sql_task::Followup::Sql { .. }) {
                    app.wide_table.h_offset = 0;
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                app.sql_task = None;
                app.request.started = None;
                app.request.activity = None;
                app.transcript.push(
                    BlockKind::Error,
                    "SQL command ended without a result.".to_string(),
                );
            }
        }
    }
}
