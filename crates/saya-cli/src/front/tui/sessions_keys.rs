//! The saved-session picker's keys (`/sessions`, bare `/resume`).
//!
//! The pane's rows, filter and confirm are [`crate::front::tui::view::Picker`]'s;
//! this module is what one press means to them and what leaves the surface. `↵`
//! resumes the selected row — the one operation `/resume <id>` states, so the
//! picker is an adapter over it rather than a second resume. `d` asks before it
//! deletes, and the ask owns the keyboard until `y` or `n`; the delete itself is
//! the shared `/sessions delete <id>` operation, never a second one here.
//!
//! The pane's rows are what the store listed when it opened, so a record the
//! store has since deleted leaves the pane the next time it opens.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::keys::Intent;
use super::model::Model;
use super::msg::Cmd;
use super::view::{Picker, SessionCommand};

/// What one picker key asks the surface to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionsIntent {
    /// Move the selection one row up.
    Up,
    /// Move the selection one row down.
    Down,
    /// Add one character to the filter.
    Type(char),
    /// Take the filter's last character back.
    Backspace,
    /// Resume the selected row.
    Open,
    /// Ask before deleting the selected row.
    Delete,
    /// Answer the ask: the row it named is deleted.
    Yes,
    /// Answer the ask: nothing is deleted.
    No,
    /// Withdraw the ask, or close the pane.
    Close,
}

/// The picker's keys: ↑↓ walk its rows, a character narrows the list,
/// Backspace takes one back, `↵` resumes the row, `d` asks before a delete and
/// Esc closes the pane. While the ask is open only `y`, `n` and Esc are read.
pub(crate) fn key(confirming: bool, key: KeyEvent) -> Option<Intent> {
    if confirming {
        let action = match key.code {
            KeyCode::Char('y') => SessionsIntent::Yes,
            KeyCode::Char('n') | KeyCode::Esc => SessionsIntent::No,
            _ => return None,
        };
        return Some(Intent::Sessions(action));
    }
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    let action = match key.code {
        KeyCode::Up => SessionsIntent::Up,
        KeyCode::Down => SessionsIntent::Down,
        KeyCode::Enter if !key.modifiers.contains(KeyModifiers::ALT) => SessionsIntent::Open,
        KeyCode::Esc => SessionsIntent::Close,
        KeyCode::Backspace => SessionsIntent::Backspace,
        KeyCode::Char('d') if !control => SessionsIntent::Delete,
        KeyCode::Char(character) if !control => SessionsIntent::Type(character),
        _ => return None,
    };
    Some(Intent::Sessions(action))
}

/// Carries one picker key out: `↵` hands the row's id to the session the surface
/// is on, `y` hands the deleted row's id to the shared delete. Nothing else
/// leaves the surface.
pub(crate) fn press(model: &mut Model, action: SessionsIntent, cmds: &mut Vec<Cmd>) {
    match action {
        SessionsIntent::Up => walk(model, Picker::up),
        SessionsIntent::Down => walk(model, Picker::down),
        SessionsIntent::Type(character) => walk(model, |picker| picker.type_char(character)),
        SessionsIntent::Backspace => walk(model, |picker| {
            picker.backspace();
        }),
        SessionsIntent::Open => {
            let chosen = model.sessions_picker_mut().and_then(Picker::resume);
            // A refused row keeps the pane open (it says why in its own row), so
            // only a row that resumed closes the surface's overlay.
            if let Some(SessionCommand::Resume { id }) = chosen {
                model.close_overlay();
                cmds.push(Cmd::ResumeSession(id));
            }
        }
        SessionsIntent::Delete => walk(model, |picker| {
            picker.ask_delete();
        }),
        SessionsIntent::Yes => {
            let chosen = model
                .sessions_picker_mut()
                .and_then(|picker| picker.answer(true));
            if let Some(SessionCommand::Delete { id }) = chosen {
                cmds.push(Cmd::Op(crate::slash::SlashCommand::SessionsDelete(id)));
            }
        }
        SessionsIntent::No => walk(model, |picker| {
            picker.answer(false);
        }),
        SessionsIntent::Close => {
            // The ask owns the first Esc; the list's own Esc closes the pane.
            let asking = model
                .sessions_picker_mut()
                .map(|picker| picker.escape().is_none() && picker.is_confirming())
                .unwrap_or(false);
            if !asking {
                model.close_overlay();
            }
        }
    }
}

/// Walks the picker with `step`, and does nothing at all when it is not open.
fn walk(model: &mut Model, step: impl FnOnce(&mut Picker)) {
    if let Some(picker) = model.sessions_picker_mut() {
        step(picker);
    }
}
