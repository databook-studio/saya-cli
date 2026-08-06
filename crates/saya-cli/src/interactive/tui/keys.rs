//! Keyboard input handling for the TUI event loop.

use super::types::App;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

/// Applies one key press to the application state.
pub(crate) fn handle_key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    // The help overlay is dismissed by any key.
    if app.show_help {
        app.show_help = false;
        return;
    }
    // F1 (or `?` on an empty line) opens the help overlay.
    if code == KeyCode::F(1) || (code == KeyCode::Char('?') && app.input.is_empty()) {
        app.show_help = true;
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
        KeyCode::F(2) => return app.toggle_selection_mode(),
        KeyCode::F(3) => return app.copy_last_answer(),
        KeyCode::F(4) => return app.copy_transcript(),
        _ => {}
    }
    // The session picker captures navigation until confirmed or cancelled.
    if app.picker.is_some() {
        match code {
            KeyCode::Up => app.picker_move(-1),
            KeyCode::Down => app.picker_move(1),
            KeyCode::Enter => app.picker_confirm(),
            KeyCode::Esc => app.picker = None,
            _ => {}
        }
        return;
    }
    // A tool-approval modal captures input until answered.
    if app.pending_approval.is_some() {
        match code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => app.answer_approval(true),
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => app.answer_approval(false),
            _ => {}
        }
        return;
    }
    // When the popup is open these keys drive it.
    if app.menu.is_some() {
        match code {
            KeyCode::Up => return app.menu_move(-1),
            KeyCode::Down => return app.menu_move(1),
            // Both Enter and Tab accept the highlighted suggestion.
            KeyCode::Tab | KeyCode::Enter => return app.accept_selected(),
            KeyCode::Esc => {
                app.menu = None;
                return;
            }
            _ => {}
        }
    }
    // Esc cancels an in-flight agent request.
    if code == KeyCode::Esc && app.is_busy() {
        if let Some(stream) = &app.stream {
            stream.cancel.cancel();
        }
        app.transcript
            .push(super::transcript::BlockKind::System, "Cancelling…");
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
            if app.is_busy() {
                if let Some(stream) = &app.stream {
                    stream.cancel.cancel();
                }
                app.transcript
                    .push(super::transcript::BlockKind::System, "Cancelling…");
            } else if !app.input.is_empty() {
                app.input.clear();
                app.menu = None;
            } else if was_armed {
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
        KeyCode::Char('d') if ctrl && app.input.is_empty() => return app.should_quit = true,
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
