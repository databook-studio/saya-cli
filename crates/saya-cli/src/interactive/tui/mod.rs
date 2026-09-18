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
mod dispatch_runs;
mod exec;
mod export;
mod fuzzy;
mod history;
mod input;
mod keys;
pub(crate) mod replay;
mod run_panel;
mod run_panel_apply;
#[cfg(test)]
mod run_panel_snapshot_tests;
#[cfg(test)]
mod run_panel_tests;
mod run_worker;
mod session_save;
mod sql_task;
mod stream_events;
mod table;
mod terminal;
pub(crate) mod transcript;
mod trust;
pub(super) mod types;
mod ui;
#[cfg(test)]
mod ui_snapshot_tests;
mod usage_footer;
mod usage_totals;

use super::session_runtime::SessionRuntime;
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

/// The TUI session's inputs: the runtime, stores, format, live session
/// state and engine, the plain-REPL trust echo (said only where the
/// plain-REPL prompt bound a directory — never on the TUI path), whether
/// the startup trust modal opens after the splash paints, and the launch's
/// host statement for a modal trust answer's recomposition. A bundle
/// rather than nine positional parameters, so the call sites read by name.
pub(crate) struct TuiSession<'a> {
    pub(crate) runtime: &'a RuntimeConfig,
    pub(crate) store: &'a FsSessionStore,
    pub(crate) state_db: &'a SqliteStateStore,
    pub(crate) format: RenderFormat,
    pub(crate) state: &'a mut SessionState,
    pub(crate) session: &'a mut SessionRuntime,
    pub(crate) trusted_echo: Option<&'a str>,
    pub(crate) trust_pending: bool,
    pub(crate) launch: &'a super::session_host::HostLaunch,
}

/// Runs the full-screen TUI session. Returns the process exit code.
///
/// `trusted_echo` carries the startup trust prompt's echo — the moment of
/// choice names the just-trusted tree beside the bypass line's lane fact.
/// `None` on every path that did not trust. `trust_pending` opens the
/// startup trust modal once after the splash paints — the TUI's rendering
/// of the one trust decision, never a raw stdin read in front of the
/// interface. `launch` recomposes the universe behind a modal trust answer
/// so the bound session carries the launch's deny list and host statement.
pub(crate) fn run(args: TuiSession<'_>) -> Result<TrustOutcome, Box<dyn std::error::Error>> {
    let TuiSession {
        runtime,
        store,
        state_db,
        format,
        state,
        session,
        trusted_echo,
        trust_pending,
        launch,
    } = args;
    let mut guard = TerminalGuard::new()?;
    let choice = runtime.resolved.output_color;
    use std::io::IsTerminal as _;
    ui::theme::set_color_enabled(ui::theme::decide_public(
        choice,
        std::io::stdout().is_terminal(),
        std::env::var_os("NO_COLOR").is_some(),
    ));
    ui::theme::set_theme(ui::theme::resolve_theme(
        runtime.resolved.ui_theme,
        std::env::var("COLORFGBG").ok().as_deref(),
    ));
    let profiles = runtime
        .connections
        .profiles
        .keys()
        .cloned()
        .collect::<Vec<String>>();
    let mut app = App::new(
        profiles,
        Arc::new(runtime.clone()),
        state_db.clone(),
        session.universe(),
    );
    // A startup fact the user must read: a pinned root that vanished, or any
    // other composition notice, said once into the transcript — the trust
    // answer's echo beside it where this launch trusted a folder — and,
    // under bypass, the mode's activation line with the probe/absence facts.
    if let Some(notice) = session.notice() {
        app.transcript.push(BlockKind::System, notice.to_string());
    }
    if let Some(echo) = trusted_echo {
        app.transcript.push(BlockKind::System, echo.to_string());
    }
    if let Some(line) =
        super::session_activation::line_if_bypass(state, runtime, &session.universe())
    {
        app.transcript.push(BlockKind::System, line);
    }
    // Bypass × unbound × non-terminal cannot reach the TUI (the TUI needs a
    // terminal), but the headless loop's twin below keeps the one wording —
    // `bypass_no_lane_note` — so the two surfaces cannot drift.
    if let Some(note) = super::session_trust::bypass_no_lane_note(
        super::session_activation::is_bypass_mode(state),
        session.universe().host_composed(),
    ) {
        app.transcript.push(BlockKind::System, note);
    }
    app.reload_at_refs(state);
    // A session resumed via --resume/--continue arrives with its turns already
    // loaded; replay them so the panel opens on the prior conversation.
    app.show_history(state);
    // The startup trust modal opens after the splash paints — never before
    // it — so the PTY splash assertion holds on every launch and the
    // question is answered inside the interface, not in front of it.
    if trust_pending {
        app.overlays.trust = Some(types::TrustPrompt::default());
    }

    // Tracks the terminal's actual mouse-capture state; TerminalGuard enables it.
    let mut mouse_captured = true;
    let mut mouse_capture_error_reported = false;
    // A modal trust answer, when one binds a directory: the live runtime
    // recomposes behind the app's snapshot once the answer lands.
    let mut trusted_dir: Option<std::path::PathBuf> = None;

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
                    let before_trust = app.overlays.trust.is_some();
                    handle_key(&mut app, key.code, key.modifiers);
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
                                    super::session_universe::SessionUniverse::compose_with_launch(
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
                                if let Some(line) = super::session_activation::line_if_bypass(
                                    state,
                                    runtime,
                                    &session.universe(),
                                ) {
                                    app.transcript.push(BlockKind::System, line);
                                }
                                trusted_dir = Some(dir);
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

        // Poll the run worker (non-blocking): the panel's step list, its
        // lifecycle line, and its episode transcript advance when the run
        // has news; the event loop never blocks on the run.
        app.poll_run_panel(state.show_thinking);

        // Poll the /compact worker (non-blocking): apply its result when ready.
        if app.compact_task.is_some() {
            super::compact_task::poll(&mut app, state);
            queue_session_save(&mut app, store, state);
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
            let id_before = state.id.clone();
            let outcome = dispatch::dispatch(
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
                    super::compact_task::start(&mut app, state);
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
            // A `/resume` swapped the session: the engine side too, so the
            // app's universe is the resumed session's, not the old one's.
            if state.id != id_before {
                app.session = session.universe();
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
                    // The resumed session's policy is its own, built from the
                    // resumed mode: grants are process-lifetime facts about
                    // one session, and a resumed session starts empty.
                    match session.reacquire(
                        runtime,
                        loaded.workspace_root.as_deref(),
                        &id,
                        loaded
                            .approval_mode
                            .parse()
                            .unwrap_or(saya_agent::ApprovalPolicy::Ask),
                    ) {
                        Ok(()) => {
                            *state = loaded;
                            app.session = session.universe();
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
                            if let Some(notice) = session.notice() {
                                app.transcript.push(BlockKind::System, notice.to_string());
                            }
                            // A resumed bypass session re-prints its
                            // activation line: the mode is real again.
                            if let Some(line) = super::session_activation::line_if_bypass(
                                state,
                                runtime,
                                &session.universe(),
                            ) {
                                app.transcript.push(BlockKind::System, line);
                            }
                        }
                        Err(error) => app.transcript.push(BlockKind::Error, error),
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

    // Session teardown: remove the chart temp files this session wrote, before
    // the terminal state is restored (DESIGN §6.6). The piped-REPL and error
    // paths drain the same registry in `session_loop`.
    crate::chart::cleanup_session_charts();

    Ok(match trusted_dir {
        Some(dir) => TrustOutcome::Answered(dir),
        None => TrustOutcome::Unasked,
    })
}

/// What the TUI's startup trust modal decided: the trusted directory when
/// the modal bound one, or `Unasked` on every other path — modal dismissed
/// unbound, or never opened. `session_loop` pins the record and refreshes
/// the header facts from the rebound session behind an `Answered`.
pub(crate) enum TrustOutcome {
    Answered(std::path::PathBuf),
    Unasked,
}

impl TrustOutcome {
    /// The process exit code: the TUI always exits cleanly here; the trust
    /// answer rides the session, never the exit status.
    pub(crate) fn exit_code(self) -> i32 {
        0
    }
}
