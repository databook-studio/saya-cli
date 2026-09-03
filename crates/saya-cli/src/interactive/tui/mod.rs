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
mod dispatch_actions;
mod dispatch_contracts;
mod exec;
mod export;
mod fuzzy;
mod history;
mod input;
mod keys;
pub(crate) mod replay;
mod session_save;
mod sql_task;
mod stream_events;
mod table;
mod terminal;
mod transcript;
pub(super) mod types;
mod ui;
#[cfg(test)]
mod ui_snapshot_tests;
mod usage_totals;

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
use saya_store::{FsSessionStore, SqliteStateStore};
use session_save::{poll_session_save, queue_session_save};
use std::sync::Arc;
use std::time::Duration;
use terminal::TerminalGuard;
use transcript::BlockKind;
use types::{App, ClipboardCopy};

/// Runs the full-screen TUI session. Returns the process exit code.
pub(crate) fn run(
    runtime: &RuntimeConfig,
    store: &FsSessionStore,
    state_db: &SqliteStateStore,
    format: RenderFormat,
    state: &mut SessionState,
) -> Result<i32, Box<dyn std::error::Error>> {
    let mut guard = TerminalGuard::new()?;
    let choice = runtime.resolved.output_color;
    use std::io::IsTerminal as _;
    ui::theme::set_color_enabled(ui::theme::decide_public(
        choice,
        std::io::stdout().is_terminal(),
        std::env::var_os("NO_COLOR").is_some(),
    ));
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

        // Advance the spinner while anything is in flight. `is_busy()` covers both an
        // agent stream and a direct-SQL command, so the status bar shows a
        // spinner while a query runs too. `drain_stream` is only
        // meaningful for an agent stream — a SQL task has no channel messages —
        // so it is gated on the stream itself.
        if app.is_busy() {
            app.spinner = app.spinner.wrapping_add(1);
            if app.request.stream.is_some() && app.drain_stream(state) {
                queue_session_save(&mut app, store, state);
            }
        }

        // Queued prompts (submitted while busy) wait until the request ends.
        if !app.is_busy()
            && let Some(line) = app.pending.take()
        {
            match dispatch::dispatch(
                &line,
                &mut app.transcript,
                &app.profiles,
                state,
                runtime,
                store,
                &app.state_db,
                format,
                &mut app.last_query,
            ) {
                Dispatch::Quit => app.should_quit = true,
                // A command may have switched profiles; refresh @-references.
                Dispatch::Handled => app.reload_at_refs(state),
                Dispatch::Agent(prompt) => app.start_agent(prompt, state),
                Dispatch::OpenSessionPicker => app.open_session_picker(store),
                Dispatch::SetColumns(arg) => app.set_visible_columns(arg),
                Dispatch::SqlTask(task) => {
                    // One SQL command in flight at a time. The queued-prompt
                    // gate (`!is_busy()`, which now covers SQL tasks) is the
                    // primary defence: a second command submitted while one
                    // runs is held until the first finishes. This guard is the
                    // backstop — should a SqlTask reach the handler while one
                    // is already running, refuse rather than silently drop the
                    // first result.
                    match app.admit_second_sql() {
                        application::SecondSqlDecision::Start => {
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
                        application::SecondSqlDecision::Reject(message) => {
                            app.transcript.push(BlockKind::System, message)
                        }
                    }
                }
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
