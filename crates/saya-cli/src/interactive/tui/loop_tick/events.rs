//! One event-loop tick's terminal-event branch.

use super::super::keys::handle_key;
use super::super::transcript::BlockKind;
use super::super::types::App;
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session::runtime::SessionRuntime;
use ratatui::crossterm::event::{Event, KeyEventKind, MouseEventKind};

/// Drains one ready terminal event. Returns `Err` only from `event::poll`/`read`.
pub(crate) fn tick_events(
    app: &mut App,
    session: &mut SessionRuntime,
    runtime: &RuntimeConfig,
    launch: &crate::interactive::session_host::HostLaunch,
    state: &mut crate::interactive::session::state::SessionState,
    trusted_dir: &mut Option<std::path::PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    use ratatui::crossterm::event::{poll, read};
    use std::time::Duration;
    if poll(Duration::from_millis(60))? {
        match read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                let before_trust = app.overlays.trust.is_some();
                handle_key(&mut *app, key.code, key.modifiers);
                apply_permission_cycle(app, state, session);
                // A modal answer that bound a directory recomposes the
                // live session behind the app's snapshot — exactly like
                // an explicit `--workspace`, through the same composer
                // with the launch's statement — and refreshes the view.
                if before_trust
                    && app.overlays.trust.is_none()
                    && let Some(dir) = app.take_trust_answer()
                {
                    match session.bind_trusted(runtime, &dir, Some(launch)) {
                        Ok(()) => {
                            app.session = session.universe();
                            if let Some(line) =
                                crate::interactive::session::permissions::activation::line_if_bypass(
                                    state,
                                    &session.universe(),
                                )
                            {
                                app.transcript.push(BlockKind::System, line);
                            }
                            *trusted_dir = Some(dir);
                        }
                        Err(error) => {
                            app.transcript.push(BlockKind::Error, error);
                        }
                    }
                }
            }
            // Bracketed paste arrives as one event, so a multi-line paste
            // lands in the input instead of submitting on the first newline.
            Event::Paste(text) => app.paste(&text),
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollUp => app.scroll_lines(true, 3),
                MouseEventKind::ScrollDown => app.scroll_lines(false, 3),
                _ => {}
            },
            _ => {}
        }
    }
    Ok(())
}

/// Drains a cycle the key handler queued. Shift+Tab runs through the one
/// shared permission-mode switch the `/mode` command runs, so the cycle is
/// not a second path: the pair, the journal and the refusal wording are the
/// switch's. It runs mid-turn as readily as between turns — the policy's mode
/// cells are shared, so an in-flight turn's decider reads the new posture at
/// its next call — and the answer lands in the transcript as a System block,
/// an Error block on a refusal (a withheld `bypass`, a frozen policy).
pub(super) fn apply_permission_cycle(
    app: &mut App,
    state: &mut crate::interactive::session::state::SessionState,
    session: &mut SessionRuntime,
) {
    if !app.pending_permission_cycle {
        return;
    }
    app.pending_permission_cycle = false;
    use crate::interactive::session::permissions::mode;
    let before = state.context_key();
    match mode::switch(state, session, mode::cycle_from(state)) {
        Ok(message) => app.transcript.push(BlockKind::System, message),
        Err(message) => app.transcript.push(BlockKind::Error, message),
    }
    state.abandon_question_when_context_changed(&before, session);
}

#[cfg(test)]
#[path = "events_tests.rs"]
mod tests;
