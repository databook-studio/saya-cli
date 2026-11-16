use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use super::super::types::App;

mod cancel;
mod editing;
mod global;
mod modals;
mod overlays;

/// Applies one key press to the application state.
pub(crate) fn handle_key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    if global::open_or_dismiss_help(app, code) {
        return;
    }
    if global::apply_global_copy_keys(app, code, mods) {
        return;
    }
    // Shift+Tab is a session-wide switch, so it is routed ahead of every
    // surface that captures the keyboard: a prompt that owns the keys would
    // otherwise swallow the press with no word about why nothing happened.
    // Where one is open the press is refused with a notice — see
    // `route_permission_cycle`.
    if global::route_permission_cycle(app, code, mods) {
        return;
    }
    if overlays::route_capture(app, code, mods) {
        return;
    }
    if modals::route(app, code, mods) {
        return;
    }
    if overlays::route_menu_popup(app, code) {
        return;
    }
    if cancel::route_escapes(app, code) {
        return;
    }
    if editing::drop_queued_prompt_chord(app, code, mods) {
        return;
    }
    if editing::apply(app, code, mods) {
        return;
    }
    // A real edit or cursor move ends history navigation, so the next Up starts fresh.
    app.history.reset();
    app.refresh_menu();
}
