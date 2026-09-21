use super::super::types::{App, SearchKind};
use super::approvals::{approval_answer, approval_choice};
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

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
    // The startup trust modal captures input until answered: one decision —
    // trust this folder, name another directory, or continue unbound — with
    // the same words the plain REPL's line prompt carries. Answering binds
    // through `SessionRuntime::bind_trusted`, exactly like `--workspace`;
    // a bad directory refuses inline (the modal stays, with the error).
    if app.overlays.trust.is_some() {
        if let Some(dir) = app.answer_trust(code, mods) {
            let _ = dir;
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
    // cancellation token, so Esc stops it cleanly. The transcript says only
    // that the stop was requested — the worker's own `Done` confirms it.
    if code == KeyCode::Esc && app.request.stream.is_some() {
        if let Some(stream) = &app.request.stream {
            stream.cancel.cancel();
        }
        app.transcript.push(
            super::super::transcript::BlockKind::System,
            "Stop requested — waiting for the worker to confirm.",
        );
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
    // Ctrl+G drops the queued prompt while a request is running. The chord
    // is free: not an approval answer (the modal returns before this point),
    // ordered after every Esc arm (Esc's SQL → stream → run-panel order is
    // untouched), not Ctrl+C (whose arm/disarm curve is unchanged), and not
    // a terminal-reserved chord. Gated on a held prompt, so without a queue
    // it falls through to normal input — nothing to drop, nothing said.
    if let KeyCode::Char('g') = code
        && ctrl
        && app.is_busy()
        && app.pending.is_some()
    {
        // Disarm like any other key: this arm returns before the shared
        // disarm below, and leaving the "press again to exit" state standing
        // across a queue drop means a later single Ctrl+C exits with no
        // second warning.
        app.ctrl_c_armed = false;
        app.drop_queued_prompt();
        return;
    }
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
                app.transcript.push(
                    super::super::transcript::BlockKind::System,
                    "Stop requested — waiting for the worker to confirm.",
                );
            } else if !app.input.is_empty() {
                app.input.clear();
                app.overlays.menu = None;
            } else if was_armed && app.try_quit() {
                app.should_quit = true;
            } else {
                app.ctrl_c_armed = true;
                app.transcript.push(
                    super::super::transcript::BlockKind::System,
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
        // Enter on an empty line toggles the most recent collapsed tool
        // group instead of submitting a blank prompt: there is no per-block
        // cursor on the transcript, so "the block at the cursor" does not
        // exist and the newest group is the one just watched stream in. A
        // bare `e` stays a typed character — stealing it would break every
        // prompt containing the letter — and Enter with any input still
        // submits. Ctrl+E (move-to-end) is untouched: this arm only fires
        // without Ctrl.
        KeyCode::Enter if app.input.is_empty() && app.toggle_tool_group() => return,
        KeyCode::Enter if app.input.is_empty() && app.toggle_latest_chapter() => return,
        KeyCode::Enter => return app.submit(),
        KeyCode::Backspace => app.input.backspace(),
        KeyCode::Delete => app.input.delete(),
        KeyCode::Left if word => app.input.move_word_left(),
        KeyCode::Right if word => app.input.move_word_right(),
        KeyCode::Left => app.input.move_left(),
        KeyCode::Right => app.input.move_right(),
        KeyCode::Home => app.input.move_home(),
        // Shift+End returns to the live edge; bare End stays input end-of-line.
        KeyCode::End if mods.contains(KeyModifiers::SHIFT) => return app.return_to_live_edge(),
        KeyCode::End => app.input.move_end(),
        KeyCode::Up | KeyCode::Down if alt => return app.step_result(code == KeyCode::Down),
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
