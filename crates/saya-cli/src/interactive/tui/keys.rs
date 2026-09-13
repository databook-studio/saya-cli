//! Keyboard input handling for the TUI event loop.

use super::types::{App, SearchKind};
use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use saya_agent::ApprovalChoice;

/// Decides the answer for a key press aimed at a pending approval modal.
/// Enter is deliberately *not* an approval: the modal can appear while the
/// user is typing, and an implicit Enter must never allow SQL to run. Only an
/// explicit `y` (or `a`) approves once; `s` grants the offered session token
/// when one exists; `n`/`d`/Esc deny; anything else is left for the modal.
pub(crate) fn approval_answer(code: KeyCode) -> Option<bool> {
    match code {
        KeyCode::Char('y') | KeyCode::Char('Y') => Some(true),
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => Some(false),
        _ => None,
    }
}

/// Decides the answer for a key press aimed at the tool-approval modal, where
/// a session grant can be offered. The `[s]` key grants exactly the token the
/// modal offered; with no token offered, `s` is not an answer at all (the
/// modal stays — it never offered a grant to take). `a` keeps allow-once
/// beside the habit keys, and Esc keeps denying.
pub(crate) fn approval_choice(code: KeyCode, grant: Option<&str>) -> Option<ApprovalChoice> {
    match code {
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Char('a') | KeyCode::Char('A') => {
            Some(ApprovalChoice::AllowOnce)
        }
        KeyCode::Char('s') | KeyCode::Char('S') => {
            grant.map(|token| ApprovalChoice::AllowSession {
                token: token.to_owned(),
            })
        }
        KeyCode::Char('n')
        | KeyCode::Char('N')
        | KeyCode::Char('d')
        | KeyCode::Char('D')
        | KeyCode::Esc => Some(ApprovalChoice::Deny),
        _ => None,
    }
}

/// Applies one key press to the application state.
pub(crate) fn handle_key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    // The help overlay is dismissed by any key.
    if app.overlays.show_help {
        app.overlays.show_help = false;
        return;
    }
    // F1 (or `?` on an empty line) opens the help overlay.
    if code == KeyCode::F(1) || (code == KeyCode::Char('?') && app.input.is_empty()) {
        app.overlays.show_help = true;
        return;
    }
    // Copy / selection keys work globally, independent of any open modal. Ctrl
    // chords are the primary bindings because macOS reserves the bare function
    // keys as media keys, so F2–F4 never reach the app there; they stay as
    // aliases for terminals that do deliver them.
    let ctrl_mod = mods.contains(KeyModifiers::CONTROL);
    match code {
        KeyCode::Char('o') if ctrl_mod => return app.toggle_selection_mode(),
        KeyCode::Char('y') if ctrl_mod => return app.copy_last_answer(),
        KeyCode::Char('b') if ctrl_mod => return app.copy_transcript(),
        KeyCode::Char('r') if ctrl_mod => return app.open_search(SearchKind::History),
        KeyCode::Char('f') if ctrl_mod => return app.open_search(SearchKind::Transcript),
        // Wide-table view: scroll left/right one column and pin the first. These
        // are view-only and never collide with input editing (the comma/period
        // only act as chords with Ctrl).
        KeyCode::Char(',') if ctrl_mod => return app.scroll_table_left(),
        KeyCode::Char('.') if ctrl_mod => return app.scroll_table_right(),
        KeyCode::Char('p') if ctrl_mod => return app.toggle_pin_first_column(),
        KeyCode::F(2) => return app.toggle_selection_mode(),
        KeyCode::F(3) => return app.copy_last_answer(),
        KeyCode::F(4) => return app.copy_transcript(),
        _ => {}
    }
    // A search overlay captures typing until committed or cancelled.
    if app.overlays.search.is_some() {
        match code {
            KeyCode::Esc => app.close_search(),
            KeyCode::Enter => app.commit_search(),
            KeyCode::Backspace => app.search_backspace(),
            KeyCode::Up => app.search_move(-1),
            KeyCode::Down => app.search_move(1),
            KeyCode::Char(c) => app.search_char(c),
            _ => {}
        }
        return;
    }
    // The session picker captures navigation and filter typing until
    // confirmed or cancelled.
    if app.overlays.picker.is_some() {
        match code {
            KeyCode::Up => app.picker_move(-1),
            KeyCode::Down => app.picker_move(1),
            KeyCode::Enter => app.picker_confirm(),
            KeyCode::Esc => app.overlays.picker = None,
            KeyCode::Backspace => app.picker_backspace(),
            KeyCode::Char(c) => app.picker_char(c),
            _ => {}
        }
        return;
    }
    // A tool-approval modal captures input until answered. The modal's
    // offered token decides whether `s` is an answer at all.
    if app.request.pending_approval.is_some() {
        let grant = app
            .request
            .pending_approval
            .as_ref()
            .and_then(|pending| pending.grant.clone());
        if let Some(choice) = approval_choice(code, grant.as_deref()) {
            app.answer_approval(choice);
        }
        return;
    }
    // The run panel's plan-approval modal captures input the same way: the
    // merged M1-10 gate, answered by the UI, never stdin.
    if app
        .run_panel
        .as_ref()
        .is_some_and(|panel| panel.plan_approval.is_some())
    {
        if let Some(allow) = approval_answer(code) {
            app.answer_plan_approval(allow);
        }
        return;
    }
    // When the popup is open these keys drive it.
    if app.overlays.menu.is_some() {
        match code {
            KeyCode::Up => return app.menu_move(-1),
            KeyCode::Down => return app.menu_move(1),
            // Both Enter and Tab accept the highlighted suggestion.
            KeyCode::Tab | KeyCode::Enter => return app.accept_selected(),
            KeyCode::Esc => {
                app.overlays.menu = None;
                return;
            }
            _ => {}
        }
    }
    // Esc on a running direct-SQL command detaches it. Checked before the agent
    // cancel path because `is_busy()` is also true while a SQL task runs, and a
    // SQL task has no cancellation token — Esc must not claim it was
    // cancelled, only that the UI moved on (see `App::detach_sql_task`).
    if code == KeyCode::Esc && app.sql_task.is_some() {
        app.detach_sql_task();
        return;
    }
    // Esc cancels an in-flight agent request. An agent stream owns a real
    // cancellation token, so Esc stops it cleanly.
    if code == KeyCode::Esc && app.request.stream.is_some() {
        if let Some(stream) = &app.request.stream {
            stream.cancel.cancel();
        }
        app.transcript
            .push(super::transcript::BlockKind::System, "Cancelling…");
        return;
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
        return;
    }
    let ctrl = mods.contains(KeyModifiers::CONTROL);
    let alt = mods.contains(KeyModifiers::ALT);
    let word = ctrl || alt;
    // Any key other than a bare Ctrl+C disarms the "press again to exit" state.
    let was_armed = app.ctrl_c_armed;
    app.ctrl_c_armed = false;
    match code {
        KeyCode::Char('c') if ctrl => {
            // A running direct-SQL command is detached (not cancelled); an
            // agent stream is cancelled; otherwise Ctrl+C clears input or arms
            // a second press to quit. The SQL check comes first because
            // `is_busy()` is true while a SQL task runs.
            if app.sql_task.is_some() {
                app.detach_sql_task();
            } else if app.request.stream.is_some() {
                if let Some(stream) = &app.request.stream {
                    stream.cancel.cancel();
                }
                app.transcript
                    .push(super::transcript::BlockKind::System, "Cancelling…");
            } else if !app.input.is_empty() {
                app.input.clear();
                app.overlays.menu = None;
            } else if was_armed && app.try_quit() {
                app.should_quit = true;
            } else {
                app.ctrl_c_armed = true;
                app.transcript.push(
                    super::transcript::BlockKind::System,
                    "Press Ctrl+C again to exit.",
                );
            }
            return;
        }
        KeyCode::Char('d') if ctrl && app.input.is_empty() => {
            return app.should_quit = app.try_quit();
        }
        KeyCode::Char('a') if ctrl => app.input.move_home(),
        KeyCode::Char('e') if ctrl => app.input.move_end(),
        KeyCode::Char('k') if ctrl => app.input.kill_to_line_end(),
        KeyCode::Char('u') if ctrl => app.input.kill_to_line_start(),
        KeyCode::Char('w') if ctrl => app.input.delete_word_left(),
        KeyCode::Char(c) if !ctrl => app.input.insert_char(c),
        KeyCode::Enter if alt || mods.contains(KeyModifiers::SHIFT) => app.input.insert_newline(),
        KeyCode::Enter => return app.submit(),
        KeyCode::Backspace => app.input.backspace(),
        KeyCode::Delete => app.input.delete(),
        KeyCode::Left if word => app.input.move_word_left(),
        KeyCode::Right if word => app.input.move_word_right(),
        KeyCode::Left => app.input.move_left(),
        KeyCode::Right => app.input.move_right(),
        KeyCode::Home => app.input.move_home(),
        KeyCode::End => app.input.move_end(),
        KeyCode::Up => return app.history_prev(),
        KeyCode::Down => return app.history_next(),
        KeyCode::PageUp => return app.scroll_pages(true),
        KeyCode::PageDown => return app.scroll_pages(false),
        _ => return,
    }
    // A real edit or cursor move ends history navigation, so the next Up starts fresh.
    app.history.reset();
    app.refresh_menu();
}

#[cfg(test)]
mod approval_modal_tests {
    use super::*;

    #[test]
    fn enter_never_approves_a_pending_modal() {
        // The modal can appear while the user is mid-thought; an implicit
        // Enter (e.g. submitting their next prompt) must never allow SQL.
        assert_eq!(approval_answer(KeyCode::Enter), None);
    }

    #[test]
    fn only_explicit_y_approves_n_and_esc_deny() {
        assert_eq!(approval_answer(KeyCode::Char('y')), Some(true));
        assert_eq!(approval_answer(KeyCode::Char('Y')), Some(true));
        assert_eq!(approval_answer(KeyCode::Char('n')), Some(false));
        assert_eq!(approval_answer(KeyCode::Char('N')), Some(false));
        assert_eq!(approval_answer(KeyCode::Esc), Some(false));
        assert_eq!(approval_answer(KeyCode::Tab), None);
    }

    /// The tool modal's three answers: `y`/`a` allow once, `s` grants exactly
    /// the offered token, `n`/`d`/Esc deny. Enter is still not an approval,
    /// and an unoffered `s` is not an answer at all — the modal never offered
    /// a grant, so the key must not invent one.
    #[test]
    fn the_tool_modal_answers_grant_only_the_offered_token() {
        use saya_agent::ApprovalChoice;
        assert_eq!(
            approval_choice(KeyCode::Char('y'), Some("workspace-write")),
            Some(ApprovalChoice::AllowOnce)
        );
        assert_eq!(
            approval_choice(KeyCode::Char('a'), None),
            Some(ApprovalChoice::AllowOnce)
        );
        assert_eq!(
            approval_choice(KeyCode::Char('s'), Some("workspace-write")),
            Some(ApprovalChoice::AllowSession {
                token: "workspace-write".to_owned()
            })
        );
        assert_eq!(
            approval_choice(KeyCode::Char('s'), None),
            None,
            "no token offered, no grant to take: the modal stays"
        );
        assert_eq!(
            approval_choice(KeyCode::Char('n'), None),
            Some(ApprovalChoice::Deny)
        );
        assert_eq!(
            approval_choice(KeyCode::Char('d'), None),
            Some(ApprovalChoice::Deny)
        );
        assert_eq!(
            approval_choice(KeyCode::Esc, None),
            Some(ApprovalChoice::Deny)
        );
        assert_eq!(
            approval_choice(KeyCode::Enter, Some("workspace-write")),
            None,
            "Enter is still never an approval"
        );
    }
}

#[cfg(test)]
mod esc_run_panel_tests {
    use super::*;
    use crate::interactive::tui::application::tests_support::idle_app;
    use crate::interactive::tui::run_panel::{RunPanel, test_channels};
    use crate::interactive::tui::run_worker::RunWorker;
    use saya_agent::CancellationToken;

    /// A panel wired the way the real worker hands one back.
    fn app_with_panel(active: bool) -> (App, CancellationToken) {
        let (_tx, rx) = test_channels();
        let cancel = CancellationToken::new();
        let mut app = idle_app();
        let mut panel = RunPanel::new(
            RunWorker {
                rx,
                cancel: cancel.clone(),
            },
            "r-keys-1".into(),
            "goal".into(),
        );
        if !active {
            panel.worker = None;
        }
        app.run_panel = Some(panel);
        (app, cancel)
    }

    /// Esc cancels an in-flight run: the token fires, the panel stays (it is
    /// cancelling), and nothing claims the conversation.
    #[test]
    fn esc_cancels_an_in_flight_run() {
        let (mut app, cancel) = app_with_panel(true);
        handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert!(cancel.is_cancelled(), "the stop cancels the token");
        assert!(app.run_panel.is_some(), "the panel stays while cancelling");
    }

    /// Esc closes a finished panel; the conversation returns and the durable
    /// record stays in /runs.
    #[test]
    fn esc_closes_a_finished_panel() {
        let (mut app, _cancel) = app_with_panel(false);
        handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert!(app.run_panel.is_none(), "the finished panel closes");
    }

    /// An agent stream in the conversation cancels first: with both a stream
    /// and a finished panel on screen, Esc stops the stream and the panel
    /// stays. A run and an agent stream are separate workers, and Esc never
    /// closes the panel out from under an active stream.
    #[test]
    fn esc_cancels_the_agent_stream_before_touching_the_panel() {
        let (mut app, _cancel) = app_with_panel(false);
        let stream =
            crate::interactive::tui::agent::start(crate::interactive::tui::agent::StreamRequest {
                runtime: app.runtime.clone(),
                prompt: "a prompt".into(),
                approval: saya_agent::ApprovalPolicy::ReadOnly,
                policy: saya_agent::SessionPolicy::new(saya_agent::ApprovalPolicy::ReadOnly),
                overrides: crate::agent::runtime::PromptOverrides::default(),
                history: Vec::new(),
                state_db: app.state_db.clone(),
                last_sql: None,
                session: std::sync::Arc::clone(&app.session),
                journal: None,
            });
        app.request.stream = Some(stream);
        handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert!(
            app.run_panel.is_some(),
            "the panel is not closed while the stream cancels"
        );
        let last = app
            .transcript
            .blocks()
            .last()
            .expect("the stream cancel posts");
        assert_eq!(last.text, "Cancelling…");
    }
}

#[cfg(test)]
mod esc_sql_task_tests {
    use super::*;
    use crate::interactive::tui::application::tests_support::idle_app_with_sql_task;
    use crate::interactive::tui::sql_task::{Followup, SqlTask};
    use crate::render::TerminalEvent;

    /// Esc while a direct-SQL command is running detaches it: the UI stops
    /// tracking the query and posts an honest "still running, result discarded"
    /// message — never "cancelled".
    #[test]
    fn esc_detaches_a_running_sql_command() {
        let mut app = idle_app_with_sql_task();
        assert!(app.sql_task.is_some(), "precondition: a task is running");
        handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert!(app.sql_task.is_none(), "Esc detached the running task");
        let last = app
            .transcript
            .blocks()
            .last()
            .expect("detach posts a message");
        let text = last.text.to_lowercase();
        assert!(
            text.contains("running"),
            "honest about still running: {text}"
        );
        assert!(
            !text.contains("cancel"),
            "must not claim cancellation: {text}"
        );
    }

    #[test]
    fn esc_does_not_detach_when_no_task_is_running() {
        let mut app = idle_app_with_sql_task();
        app.sql_task = None;
        handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert!(app.sql_task.is_none());
        // No detach message posted (the app starts with an empty transcript).
        assert!(app.transcript.blocks().is_empty());
    }

    #[test]
    fn in_flight_task_shape_matches_app_state() {
        // Guards the tuple arity (receiver, task, instant) the dispatch loop
        // and detach path destructure.
        let (_tx, rx) = std::sync::mpsc::channel::<TerminalEvent>();
        let task = SqlTask {
            profile: Some("analytics".into()),
            sql: "SELECT 1".into(),
            followup: Followup::Sql {
                connection: Some("analytics".into()),
            },
        };
        let _: (
            std::sync::mpsc::Receiver<TerminalEvent>,
            SqlTask,
            std::time::Instant,
        ) = (rx, task, std::time::Instant::now());
    }
}
