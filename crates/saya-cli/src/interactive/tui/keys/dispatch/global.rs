use super::super::super::transcript::BlockKind;
use super::super::super::types::{App, SearchKind};
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

/// The refusal a Shift+Tab answers with while a prompt owns the keyboard.
/// Short, and true about what happened: nothing moved, and the reason is the
/// open prompt, not the mode.
const CYCLE_REFUSED: &str = "Permission mode unchanged — a prompt is open. Answer or dismiss it, then press Shift+Tab again.";

/// Whether a surface that owns the keyboard is open, so a permission switch
/// would move the posture under a decision the user is still giving. The
/// completion popup is deliberately not one: it is an input affordance, and
/// Shift+Tab beside it is still the user's switch.
fn keyboard_owned(app: &App) -> bool {
    app.request.pending_approval.is_some()
        || app
            .run_panel
            .as_ref()
            .is_some_and(|panel| panel.plan_approval.is_some())
        || app.overlays.trust.is_some()
        || app.overlays.search.is_some()
        || app.overlays.picker.is_some()
        || app.overlays.investigations.is_some()
}

/// The permission-mode cycle's key — Shift+Tab, which terminals report as
/// `BackTab` (`Esc [ Z`) or as `Tab` with the shift modifier. Both spellings
/// are the one press, and both are routed ahead of the popup that also
/// answers `Tab`; `true` when the key was consumed.
///
/// The press only queues the intent: the switch needs the session's one
/// policy and its journal, which this layer does not hold, so the same tick's
/// event loop drains [`App::pending_permission_cycle`] and runs it.
pub(super) fn route_permission_cycle(app: &mut App, code: KeyCode, mods: KeyModifiers) -> bool {
    let cycle =
        code == KeyCode::BackTab || (code == KeyCode::Tab && mods.contains(KeyModifiers::SHIFT));
    if !cycle {
        return false;
    }
    if keyboard_owned(app) {
        app.transcript.push(BlockKind::System, CYCLE_REFUSED);
        return true;
    }
    app.pending_permission_cycle = true;
    true
}

/// The help overlay's open and dismiss keys; `true` when the key was
/// consumed.
pub(super) fn open_or_dismiss_help(app: &mut App, code: KeyCode) -> bool {
    // The help overlay is dismissed by any key.
    if app.overlays.show_help {
        app.overlays.show_help = false;
        return true;
    }
    // F1 (or `?` on an empty line) opens the help overlay.
    if code == KeyCode::F(1) || (code == KeyCode::Char('?') && app.input.is_empty()) {
        app.overlays.show_help = true;
        return true;
    }
    false
}

/// The global copy / selection chords, independent of any open modal;
/// `true` when the key was consumed.
pub(super) fn apply_global_copy_keys(app: &mut App, code: KeyCode, mods: KeyModifiers) -> bool {
    // Copy / selection keys work globally, independent of any open modal. Ctrl
    // chords are the primary bindings because macOS reserves the bare function
    // keys as media keys, so F2–F4 never reach the app there; they stay as
    // aliases for terminals that do deliver them.
    let ctrl_mod = mods.contains(KeyModifiers::CONTROL);
    match code {
        KeyCode::Char('o') if ctrl_mod => {
            app.toggle_selection_mode();
            return true;
        }
        KeyCode::Char('y') if ctrl_mod => {
            app.copy_last_answer();
            return true;
        }
        KeyCode::Char('b') if ctrl_mod => {
            app.copy_transcript();
            return true;
        }
        KeyCode::Char('r') if ctrl_mod => {
            app.open_search(SearchKind::History);
            return true;
        }
        KeyCode::Char('f') if ctrl_mod => {
            app.open_search(SearchKind::Transcript);
            return true;
        }
        // Wide-table view: scroll left/right one column and pin the first. These
        // are view-only and never collide with input editing (the comma/period
        // only act as chords with Ctrl).
        KeyCode::Char(',') if ctrl_mod => {
            app.scroll_table_left();
            return true;
        }
        KeyCode::Char('.') if ctrl_mod => {
            app.scroll_table_right();
            return true;
        }
        KeyCode::Char('p') if ctrl_mod => {
            app.toggle_pin_first_column();
            return true;
        }
        KeyCode::F(2) => {
            app.toggle_selection_mode();
            return true;
        }
        KeyCode::F(3) => {
            app.copy_last_answer();
            return true;
        }
        KeyCode::F(4) => {
            app.copy_transcript();
            return true;
        }
        _ => {}
    }
    false
}
