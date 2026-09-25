//! Advancing the spinner and draining the agent stream while in flight.

use super::super::session_save::queue_session_save;
use super::super::types::App;
use crate::interactive::session_state::SessionState;
use saya_store::FsSessionStore;

// Advance the spinner while anything is in flight. `is_busy()` covers both an
// agent stream and a direct-SQL command, so the status bar shows a
// spinner while a query runs too. `drain_stream` is only
// meaningful for an agent stream — a SQL task has no channel messages —
// so it is gated on the stream itself.
pub(crate) fn tick_busy(app: &mut App, store: &FsSessionStore, state: &mut SessionState) {
    if app.is_busy() {
        app.spinner = app.spinner.wrapping_add(1);
        if app.request.stream.is_some() && app.drain_stream(state) {
            queue_session_save(&mut *app, store, state);
        }
    }
}
