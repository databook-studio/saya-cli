use super::super::super::transcript::BlockKind;
use super::super::super::types::App;
use super::cancel::request_stream_stop;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

/// The Ctrl+G queue-drop chord while a request runs; `true` when the chord
/// was consumed.
pub(super) fn drop_queued_prompt_chord(app: &mut App, code: KeyCode, mods: KeyModifiers) -> bool {
    let ctrl = mods.contains(KeyModifiers::CONTROL);
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
        return true;
    }
    false
}

/// The editing and navigation keys — input editing, Ctrl+C arm/disarm,
/// history, and scrolling; `true` when the key was consumed, so a fall
/// through ends history navigation and refreshes the menu.
pub(super) fn apply(app: &mut App, code: KeyCode, mods: KeyModifiers) -> bool {
    // Any key other than a bare Ctrl+C disarms the "press again to exit" state.
    let was_armed = app.ctrl_c_armed;
    app.ctrl_c_armed = false;
    let ctrl = mods.contains(KeyModifiers::CONTROL);
    let alt = mods.contains(KeyModifiers::ALT);
    let word = ctrl || alt;
    match code {
        KeyCode::Char('c') if ctrl => {
            // A running direct-SQL command receives a cancellation request and
            // detaches; replay detaches; agent streams cancel. Otherwise Ctrl+C
            // clears input or arms a quit. Query checks come first because
            // `is_busy()` covers those workers.
            if app.detach_in_flight_query() {
                return true;
            }
            if app.request.stream.is_some() {
                if let Some(stream) = &app.request.stream {
                    request_stream_stop(stream);
                }
                app.transcript.push(
                    BlockKind::System,
                    "Stop requested — waiting for the worker to confirm.",
                );
                return true;
            }
            if !app.input.is_empty() {
                app.input.clear();
                app.overlays.menu = None;
                return true;
            }
            if was_armed && app.try_quit() {
                app.should_quit = true;
                return true;
            }
            app.ctrl_c_armed = true;
            app.transcript
                .push(BlockKind::System, "Press Ctrl+C again to exit.");
        }
        KeyCode::Char('d') if ctrl && app.input.is_empty() => {
            app.should_quit = app.try_quit();
            return true;
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
        KeyCode::Enter if app.input.is_empty() && app.toggle_tool_group() => return true,
        KeyCode::Enter if app.input.is_empty() && app.toggle_latest_chapter() => return true,
        KeyCode::Enter => {
            app.submit();
            return true;
        }
        KeyCode::Backspace => app.input.backspace(),
        KeyCode::Delete => app.input.delete(),
        KeyCode::Left if word => app.input.move_word_left(),
        KeyCode::Right if word => app.input.move_word_right(),
        KeyCode::Left => app.input.move_left(),
        KeyCode::Right => app.input.move_right(),
        KeyCode::Home => app.input.move_home(),
        // Shift+End returns to the live edge; bare End stays input end-of-line.
        KeyCode::End if mods.contains(KeyModifiers::SHIFT) => {
            app.return_to_live_edge();
            return true;
        }
        KeyCode::End => app.input.move_end(),
        KeyCode::Up | KeyCode::Down if alt => {
            app.step_result(code == KeyCode::Down);
            return true;
        }
        KeyCode::Up => {
            app.history_prev();
            return true;
        }
        KeyCode::Down => {
            app.history_next();
            return true;
        }
        KeyCode::PageUp => {
            app.scroll_pages(true);
            return true;
        }
        KeyCode::PageDown => {
            app.scroll_pages(false);
            return true;
        }
        _ => return true,
    }
    false
}
