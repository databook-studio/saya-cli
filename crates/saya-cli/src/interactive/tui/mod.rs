//! Full-screen terminal UI for the interactive session.
//!
//! A scrolling transcript region on top, a one-line status bar, a bordered
//! multi-line input box pinned to the bottom, and a slash-command popup that
//! opens the instant the line starts with '/' and filters as you type. Slash
//! commands and `/sql` execute and render into the transcript; live agent
//! streaming arrives in a later milestone. Non-TTY input uses a headless
//! executor, not this module. Rendering lives in `ui`.

mod agent;
mod application;
mod atref;
mod clipboard;
mod complete;
mod dispatch;
mod exec;
mod fuzzy;
mod history;
mod input;
mod keys;
mod replay;
mod stream_events;
mod table;
mod terminal;
mod transcript;
mod types;
mod ui;

use super::session_resume::block_on;
use super::session_state::SessionState;
use crate::config::runtime::RuntimeConfig;
use crate::render::RenderFormat;
use clipboard::{copy_to_native_clipboard, osc52_copy};
use dispatch::Dispatch;
use keys::handle_key;
use ratatui::crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind, MouseEventKind},
    execute,
};
use saya_store::{FsSessionStore, SessionStore, SqliteStateStore};
use std::sync::Arc;
use std::time::Duration;
use terminal::TerminalGuard;
use transcript::{BlockKind, Transcript};
use types::App;

fn save_session(store: &FsSessionStore, state: &SessionState, transcript: &mut Transcript) {
    if let Err(error) = block_on(store.save(state.redacted())) {
        transcript.push(
            BlockKind::Error,
            format!("Could not save this session; your latest changes may be lost: {error}"),
        );
    }
}

/// Runs the full-screen TUI session. Returns the process exit code.
pub(crate) fn run(
    runtime: &RuntimeConfig,
    store: &FsSessionStore,
    state_db: &SqliteStateStore,
    format: RenderFormat,
    state: &mut SessionState,
) -> Result<i32, Box<dyn std::error::Error>> {
    let mut guard = TerminalGuard::new()?;
    let profiles = runtime
        .connections
        .profiles
        .keys()
        .cloned()
        .collect::<Vec<String>>();
    let mut app = App::new(profiles, Arc::new(runtime.clone()), state_db.clone());
    app.reload_at_refs(state);
    // A session resumed via --resume/--continue arrives with its turns already
    // loaded; replay them so the panel opens on the prior conversation.
    app.show_history(state);

    // Tracks the terminal's actual mouse-capture state; TerminalGuard enables it.
    let mut mouse_captured = true;
    let mut mouse_capture_error_reported = false;

    while !app.should_quit {
        let status = super::session_prompt::status_segments(state);
        guard
            .terminal
            .draw(|frame| ui::draw(frame, &app, &status))?;

        if event::poll(Duration::from_millis(60))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    handle_key(&mut app, key.code, key.modifiers)
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

        // Reconcile the terminal's mouse capture with selection mode: releasing
        // capture lets the terminal drag-select and copy; re-grabbing it restores
        // wheel scrolling.
        let want_capture = !app.selection_mode;
        if want_capture != mouse_captured {
            let backend = guard.terminal.backend_mut();
            let result = if want_capture {
                execute!(backend, EnableMouseCapture)
            } else {
                execute!(backend, DisableMouseCapture)
            };
            match result {
                Ok(()) => {
                    mouse_captured = want_capture;
                    mouse_capture_error_reported = false;
                }
                Err(error) if !mouse_capture_error_reported => {
                    app.transcript.push(
                        BlockKind::Error,
                        format!("Could not update mouse capture: {error}"),
                    );
                    mouse_capture_error_reported = true;
                }
                Err(_) => {}
            }
        }

        // Fulfil a queued clipboard copy. Try the OS clipboard tool first (pbcopy
        // / wl-copy / xclip / clip) since that's what actually works locally —
        // notably in macOS Terminal.app, which ignores OSC 52. Always also emit
        // OSC 52 so copies can still reach the *local* clipboard over SSH. Report
        // the outcome only after attempting both mechanisms.
        if let Some(text) = app.pending_clipboard.take() {
            let native_ok = copy_to_native_clipboard(&text);
            let osc_result = osc52_copy(guard.terminal.backend_mut(), &text);
            match (native_ok, osc_result) {
                (true, _) => app
                    .transcript
                    .push(BlockKind::System, "Copied to the system clipboard."),
                (false, Ok(())) => app.transcript.push(
                    BlockKind::System,
                    "Sent OSC 52 clipboard data; it will copy if your terminal supports it.",
                ),
                (false, Err(error)) => app.transcript.push(
                    BlockKind::Error,
                    format!(
                        "Could not copy to the system clipboard, and sending OSC 52 failed: {error}"
                    ),
                ),
            }
        }

        if app.is_busy() {
            app.spinner = app.spinner.wrapping_add(1);
            if app.drain_stream(state) {
                save_session(store, state, &mut app.transcript);
            }
        }

        if let Some(line) = app.pending.take() {
            match dispatch::dispatch(
                &line,
                &mut app.transcript,
                &app.profiles,
                state,
                runtime,
                store,
                format,
            ) {
                Dispatch::Quit => app.should_quit = true,
                // A command may have switched profiles; refresh @-references.
                Dispatch::Handled => app.reload_at_refs(state),
                Dispatch::Agent(prompt) => app.start_agent(prompt, state),
                Dispatch::OpenSessionPicker => app.open_session_picker(store),
            }
        }

        if let Some(id) = app.pending_resume.take() {
            let defaults = super::session_resume::SessionDefaults {
                provider: state.provider.clone(),
                model: state.model.clone(),
                allow_data_sharing: state.allow_data_sharing,
                approval_mode: state.approval_mode.clone(),
            };
            match super::session_resume::resume_session(store, &id, &defaults) {
                Ok(Some(loaded)) => {
                    *state = loaded;
                    app.reload_at_refs(state);
                    if state.turns.is_empty() {
                        app.transcript.clear();
                        app.transcript.push(
                            BlockKind::System,
                            format!("Resumed session {id} (no earlier turns)."),
                        );
                    } else {
                        // Replace the panel with the resumed session's conversation.
                        app.show_history(state);
                    }
                }
                Ok(None) => app
                    .transcript
                    .push(BlockKind::Error, format!("Session not found: {id}")),
                Err(error) => app.transcript.push(BlockKind::Error, error.to_string()),
            }
            save_session(store, state, &mut app.transcript);
        }
    }

    Ok(0)
}
