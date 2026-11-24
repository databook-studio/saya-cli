//! The keys the panes that own the screen read.
//!
//! A pane takes the keyboard before the card and the composer (`02` §8, §11), so
//! its presses are read here and nowhere else: what `↑` means depends on which
//! pane is up, and the composer behind it never sees the key.
//!
//! Every row of the key sheet is checked against this module by
//! `view::help_tests`: a pane that stopped answering a listed key fails the
//! suite instead of leaving the sheet claiming a keyboard the surface lost.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::direct_trust;
use super::keys::Intent;
use super::model::Model;
use super::model_overlay::Overlay;
use crate::interactive::tui::TrustResolution;

/// What `key` means to the pane that is open.
pub(crate) fn key(model: &Model, overlay: &Overlay, key: KeyEvent) -> Option<Intent> {
    match overlay {
        Overlay::Mode(_) => mode(key),
        Overlay::Palette(_) => palette(key),
        Overlay::Help => help(key),
        Overlay::Jobs => jobs(model.jobs().is_confirming(), key),
        Overlay::Sessions(picker) => super::sessions_keys::key(picker.is_confirming(), key),
        Overlay::Changes(_) => changes(key),
        Overlay::Trust => trust(key),
    }
}

/// `/mode`'s picker: ↑↓ choose, Enter set, Esc cancel (`02` §8).
fn mode(key: KeyEvent) -> Option<Intent> {
    match key.code {
        KeyCode::Up => Some(Intent::ModeUp),
        KeyCode::Down => Some(Intent::ModeDown),
        KeyCode::Enter if !key.modifiers.contains(KeyModifiers::ALT) => Some(Intent::ModeSet),
        KeyCode::Esc => Some(Intent::ModeCancel),
        _ => None,
    }
}

/// The command palette (`02` §11): a typed character narrows the list, ↑↓ choose
/// a row, Enter runs or fills the selected command and Esc closes the pane. The
/// character arm is last, so no control chord can be typed into the query.
fn palette(key: KeyEvent) -> Option<Intent> {
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Up => Some(Intent::PaletteUp),
        KeyCode::Down => Some(Intent::PaletteDown),
        KeyCode::Enter if !key.modifiers.contains(KeyModifiers::ALT) => Some(Intent::PaletteRun),
        KeyCode::Esc => Some(Intent::PaletteClose),
        KeyCode::Backspace => Some(Intent::PaletteBackspace),
        KeyCode::Char(character) if !control => Some(Intent::PaletteInsert(character)),
        _ => None,
    }
}

/// The key sheet (`02` §11): it says what the keys do, Esc closes it, and
/// ↑↓ and PgUp/PgDn scroll its body when the screen is shorter than the sheet.
fn help(key: KeyEvent) -> Option<Intent> {
    match key.code {
        KeyCode::Esc => Some(Intent::HelpClose),
        KeyCode::Up => Some(Intent::HelpUp),
        KeyCode::Down => Some(Intent::HelpDown),
        KeyCode::PageUp => Some(Intent::HelpPageUp),
        KeyCode::PageDown => Some(Intent::HelpPageDown),
        _ => None,
    }
}

/// The jobs drawer's keys (`02` §8): `↑↓` walk its rows, `↵` opens the selected
/// job, `a` opens the card for the call it is waiting on, `x` asks before it
/// cancels, and Esc closes the drawer. While the confirm asks, only `y`, `n` and
/// Esc are read — the ask owns the keyboard, exactly as T12's `d` does.
fn jobs(confirming: bool, key: KeyEvent) -> Option<Intent> {
    if confirming {
        return match key.code {
            KeyCode::Char('y') => Some(Intent::JobsCancelYes),
            KeyCode::Char('n') => Some(Intent::JobsCancelNo),
            KeyCode::Esc => Some(Intent::JobsCancelNo),
            _ => None,
        };
    }
    match key.code {
        KeyCode::Up => Some(Intent::JobsUp),
        KeyCode::Down => Some(Intent::JobsDown),
        KeyCode::Enter if !key.modifiers.contains(KeyModifiers::ALT) => Some(Intent::JobsOpen),
        KeyCode::Char('a') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Intent::JobsApprove)
        }
        KeyCode::Char('x') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Intent::JobsCancelAsk)
        }
        KeyCode::Esc => Some(Intent::JobsClose),
        _ => None,
    }
}

/// The changes pane (`Ctrl+D`, UI5): ↑↓ walk the rows, the page keys walk a
/// screen at a time, and Esc closes it. The composer behind it never sees a
/// press, exactly as the other panes' keys work.
fn changes(key: KeyEvent) -> Option<Intent> {
    match key.code {
        KeyCode::Up => Some(Intent::ChangesUp),
        KeyCode::Down => Some(Intent::ChangesDown),
        KeyCode::PageUp => Some(Intent::ChangesPageUp),
        KeyCode::PageDown => Some(Intent::ChangesPageDown),
        KeyCode::Esc => Some(Intent::ChangesClose),
        _ => None,
    }
}

/// The workspace-trust question's keys (`02` §9.4): `y` binds the launch folder
/// and `n` continues without one — the two answers today's modal gives through
/// `t` and `c`, read through the same resolution (`direct_trust::press`).
fn trust(key: KeyEvent) -> Option<Intent> {
    match direct_trust::press(key.code)? {
        TrustResolution::TrustCwd => Some(Intent::TrustAccept),
        _ => Some(Intent::TrustDecline),
    }
}
