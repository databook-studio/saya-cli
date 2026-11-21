//! The keymap: what one key press means to the model, in the state it is in.
//!
//! A press is read here and nowhere else, so the same key can mean different
//! things without the update growing a decision tree: Esc answers an open card
//! and otherwise stops a running turn, and Shift+Tab is refused outright while
//! a card waits — a mode switch the operator asked for mid-answer would leave
//! the question they were reading behind.
//!
//! The card owns the keyboard while it is open (its answer keys land with the
//! card's own unit); the composer owns it otherwise.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::model::Model;

/// What the surface says when a mode switch is asked for over an open card.
/// The card asks for an answer first, and the wording says so.
pub(crate) const CARD_HOLDS_MODE: &str = "Shift+Tab ignored — answer the approval first";

/// What one press asks the model to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Intent {
    /// Put the character at the composer's end.
    Insert(char),
    /// Break the line: Alt+Enter, so a bare Enter stays "send".
    Newline,
    /// Remove the composer's last character.
    Backspace,
    /// Send the composer's text as a turn.
    Submit,
    /// Stop the turn in flight; the run stays open.
    CancelTurn,
    /// Deny the call the open card asks about.
    DenyCard,
    /// Walk the mode cycle to its next member.
    CycleMode,
    /// Say this instead of doing anything: the state the press met refuses it.
    Refused(&'static str),
}

/// What `key` means to `model` now, or nothing at all.
pub(crate) fn intent(model: &Model, key: KeyEvent) -> Option<Intent> {
    if model.card.is_some() {
        return card_key(key);
    }
    composer_key(model, key)
}

/// The card's keys: a denial, and a refusal of the one global key that would
/// otherwise act on a state the operator cannot see behind the card. The
/// answer keys (`y`, `a`, `s`, `n`, `d`, Esc) belong to the card's own unit;
/// Enter is deliberately none of them, so a press meant for the composer
/// during a card can never consent.
fn card_key(key: KeyEvent) -> Option<Intent> {
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    match key.code {
        KeyCode::Esc => Some(Intent::DenyCard),
        KeyCode::BackTab => Some(Intent::Refused(CARD_HOLDS_MODE)),
        KeyCode::Tab if shift => Some(Intent::Refused(CARD_HOLDS_MODE)),
        _ => None,
    }
}

/// The composer's keys.
fn composer_key(model: &Model, key: KeyEvent) -> Option<Intent> {
    let modifiers = key.modifiers;
    let alt = modifiers.contains(KeyModifiers::ALT);
    let ctrl = modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Enter if alt => Some(Intent::Newline),
        KeyCode::Enter if !ctrl => Some(Intent::Submit),
        KeyCode::BackTab => Some(Intent::CycleMode),
        KeyCode::Tab if modifiers.contains(KeyModifiers::SHIFT) => Some(Intent::CycleMode),
        KeyCode::Esc if model.is_busy() => Some(Intent::CancelTurn),
        KeyCode::Backspace => Some(Intent::Backspace),
        KeyCode::Char(character) if !ctrl && !alt => Some(Intent::Insert(character)),
        _ => None,
    }
}
