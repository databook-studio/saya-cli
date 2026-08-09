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
use clipboard::{ClipboardOutcome, clipboard_outcome, copy_to_native_clipboard, osc52_copy};
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
use transcript::BlockKind;
use types::{App, ClipboardCopy, SessionSave};

fn start_session_save(app: &mut App, store: &FsSessionStore, session: saya_store::RedactedSession) {
    let store = store.clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = block_on(store.save(session)).map_err(|error| error.to_string());
        let _ = sender.send(result);
    });
    app.session_save = Some(SessionSave { result: receiver });
}

fn queue_session_save(app: &mut App, store: &FsSessionStore, state: &SessionState) {
    let session = state.redacted();
    if app.session_save.is_some() {
        app.pending_session_save = Some(session);
    } else {
        start_session_save(app, store, session);
    }
}

fn poll_session_save(app: &mut App, store: &FsSessionStore) {
    let result = app
        .session_save
        .as_ref()
        .and_then(|save| match save.result.try_recv() {
            Ok(result) => Some(result),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Some(Err("session save worker stopped unexpectedly".into()))
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
        });
    let Some(result) = result else { return };
    app.session_save = None;
    if let Err(error) = result {
        app.transcript.push(
            BlockKind::Error,
            format!("Could not save this session; your latest changes may be lost: {error}"),
        );
    }
    if let Some(session) = app.pending_session_save.take() {
        start_session_save(app, store, session);
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
        app.poll_session_picker();
        poll_session_save(&mut app, store);
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
        let want_capture = !app.overlays.selection_mode;
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

        // Fulfil queued clipboard copies without blocking the event loop. Try the OS clipboard tool first (pbcopy
        // / wl-copy / xclip / clip) since that's what actually works locally —
        // notably in macOS Terminal.app, which ignores OSC 52. Always also emit
        // OSC 52 so copies can still reach the *local* clipboard over SSH. Report
        // the outcome only after attempting both mechanisms.
        if app.clipboard_copy.is_none()
            && let Some(text) = app.pending_clipboard.take()
        {
            let (sender, receiver) = std::sync::mpsc::channel();
            let native_text = text.clone();
            std::thread::spawn(move || {
                let _ = sender.send(copy_to_native_clipboard(&native_text));
            });
            app.clipboard_copy = Some(ClipboardCopy {
                native_result: receiver,
                osc_error: osc52_copy(guard.terminal.backend_mut(), &text)
                    .err()
                    .map(|error| error.to_string()),
            });
        }
        let native_result =
            app.clipboard_copy
                .as_ref()
                .and_then(|copy| match copy.native_result.try_recv() {
                    Ok(result) => Some(result),
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(false),
                    Err(std::sync::mpsc::TryRecvError::Empty) => None,
                });
        if let Some(native_ok) = native_result
            && let Some(copy) = app.clipboard_copy.take()
        {
            match clipboard_outcome(native_ok, copy.osc_error.as_deref()) {
                ClipboardOutcome::Native => app
                    .transcript
                    .push(BlockKind::System, "Copied to the system clipboard."),
                ClipboardOutcome::Osc52 => app.transcript.push(
                    BlockKind::System,
                    "Sent OSC 52 clipboard data; it will copy if your terminal supports it.",
                ),
                ClipboardOutcome::Failed => app.transcript.push(
                    BlockKind::Error,
                    format!(
                        "Could not copy to the system clipboard, and sending OSC 52 failed: {}",
                        copy.osc_error
                            .unwrap_or_else(|| "unknown terminal output error".into())
                    ),
                ),
            }
        }

        if app.is_busy() {
            app.spinner = app.spinner.wrapping_add(1);
            if app.drain_stream(state) {
                queue_session_save(&mut app, store, state);
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
            queue_session_save(&mut app, store, state);
        }

        if let Some(id) = app.overlays.pending_resume.take() {
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
            queue_session_save(&mut app, store, state);
        }
    }

    Ok(0)
}
