use super::super::super::agent::messages::Stream;
use super::super::super::transcript::BlockKind;
use super::super::super::types::App;
use ratatui::crossterm::event::KeyCode;

/// Requests the answering stream's stop through the turn's question scope
/// when it carries one — the scope captures the application-side stop
/// instant, first stop wins — and falls back to the bare token otherwise.
pub(super) fn request_stream_stop(stream: &Stream) {
    match &stream.question_scope {
        Some(scope) => scope.request_stop(),
        None => stream.cancel.cancel(),
    }
}

/// The Esc arms — detach an in-flight SQL task or replay, cancel an agent
/// stream, close or cancel the run panel; `true` when the key was consumed.
pub(super) fn route_escapes(app: &mut App, code: KeyCode) -> bool {
    // Esc requests cancellation for direct SQL and detaches it, or detaches a
    // saved-investigation replay — both checks precede the agent cancel path.
    if code == KeyCode::Esc && app.detach_in_flight_query() {
        return true;
    }
    // Esc cancels an in-flight agent request. An agent stream owns a real
    // cancellation token, so Esc stops it cleanly. The transcript says only
    // that the stop was requested — the worker's own `Done` confirms it.
    if code == KeyCode::Esc && app.request.stream.is_some() {
        if let Some(stream) = &app.request.stream {
            request_stream_stop(stream);
        }
        app.transcript.push(
            BlockKind::System,
            "Stop requested — waiting for the worker to confirm.",
        );
        return true;
    }
    // Esc cancels the panel's in-flight run — the token cancels and the
    // worker records the stop through the engine path `saya run cancel`
    // takes. When the panel's run is over, Esc closes the panel; the
    // conversation returns and the durable record stays in /runs. Checked
    // after the SQL detach and agent cancel — an agent stream in the
    // conversation cancels first — and never races them: a run and those
    // are separate workers.
    if code == KeyCode::Esc
        && let Some(panel) = app.run_panel.as_ref()
    {
        if panel.is_active() {
            app.cancel_run_panel();
        } else {
            app.close_run_panel();
        }
        return true;
    }
    false
}
