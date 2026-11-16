use super::super::super::types::App;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

/// The capture overlays — search, session picker, investigations picker —
/// which consume every key while open; `true` when the key was consumed.
pub(super) fn route_capture(app: &mut App, code: KeyCode, mods: KeyModifiers) -> bool {
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
        return true;
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
        return true;
    }
    // The investigation picker captures navigation and filter typing until
    // the choice is shown, run, or cancelled. `r` runs the selected one (a
    // filter containing a bare `r` is not typeable here — the trade every
    // single-key action on this surface makes); Ctrl/Alt chords are never
    // the run key.
    if app.overlays.investigations.is_some() {
        match code {
            KeyCode::Up => app.investigations_move(-1),
            KeyCode::Down => app.investigations_move(1),
            KeyCode::Enter => app.investigations_confirm(),
            KeyCode::Esc => app.overlays.investigations = None,
            KeyCode::Backspace => app.investigations_backspace(),
            KeyCode::Char('r') if !mods.contains(KeyModifiers::CONTROL) => {
                app.investigations_run();
            }
            KeyCode::Char(c) => app.investigations_char(c),
            _ => {}
        }
        return true;
    }
    false
}

/// The menu popup's driving keys; `true` when the key was consumed, so an
/// unmatched key falls through to the Esc arms and editing below.
pub(super) fn route_menu_popup(app: &mut App, code: KeyCode) -> bool {
    // When the popup is open these keys drive it.
    if app.overlays.menu.is_some() {
        match code {
            KeyCode::Up => {
                app.menu_move(-1);
                return true;
            }
            KeyCode::Down => {
                app.menu_move(1);
                return true;
            }
            // Both Enter and Tab accept the highlighted suggestion.
            KeyCode::Tab | KeyCode::Enter => {
                app.accept_selected();
                return true;
            }
            KeyCode::Esc => {
                app.overlays.menu = None;
                return true;
            }
            _ => {}
        }
    }
    false
}
