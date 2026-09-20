//! One event-loop tick's terminal-event branch.

use super::super::keys::handle_key;
use super::super::transcript::BlockKind;
use super::super::types::App;
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session_runtime::SessionRuntime;
use ratatui::crossterm::event::{Event, KeyEventKind, MouseEventKind};

/// Drains one ready terminal event. Returns `Err` only from `event::poll`/`read`.
pub(crate) fn tick_events(
    app: &mut App,
    session: &mut SessionRuntime,
    runtime: &RuntimeConfig,
    launch: &crate::interactive::session_host::HostLaunch,
    state: &mut crate::interactive::session_state::SessionState,
    trusted_dir: &mut Option<std::path::PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    use ratatui::crossterm::event::{poll, read};
    use std::time::Duration;
    if poll(Duration::from_millis(60))? {
        match read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                let before_trust = app.overlays.trust.is_some();
                handle_key(&mut *app, key.code, key.modifiers);
                // A modal answer that bound a directory recomposes the
                // live session behind the app's snapshot — exactly like
                // an explicit `--workspace`, through the same composer
                // with the launch's statement — and refreshes the view.
                if before_trust
                    && app.overlays.trust.is_none()
                    && let Some(dir) = app.take_trust_answer()
                {
                    match session.bind_trusted(runtime, &dir) {
                        Ok(()) => {
                            let recomposed =
                                    crate::interactive::session_universe::SessionUniverse::compose_with_launch(
                                        runtime,
                                        session.explicit_statement(),
                                        state.workspace_root.as_deref(),
                                        true,
                                        &std::env::current_dir()
                                            .unwrap_or_else(|_| std::path::PathBuf::from(".")),
                                        &session.state_dir(),
                                        Some(launch),
                                    )?;
                            session.replace_universe(recomposed);
                            app.session = session.universe();
                            if let Some(line) =
                                crate::interactive::session_activation::line_if_bypass(
                                    state,
                                    runtime,
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
