//! The keymap: what one key press means to the model, in the state it is in.
//!
//! A press is read here and nowhere else, so the same key can mean different
//! things without the update growing a decision tree: Esc answers an open card
//! and otherwise stops a running turn, and Shift+Tab is refused outright while
//! a card waits — a mode switch the operator asked for mid-answer would leave
//! the question they were reading behind.
//!
//! The card owns the keyboard while it is open, and its own answers are read
//! there ([`super::cards_keys`]); the composer owns it otherwise.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::cards_keys::{self, CardIntent};
use super::model::Model;
use crate::agent::tools::consent::Answer;

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
    /// Drop the line held behind the running turn: Ctrl+G drops, never sends
    /// (`02` §7).
    DropQueued,
    /// Stop the turn in flight; the run stays open.
    CancelTurn,
    /// Answer the call the open card asks about with this.
    AnswerCard(Answer),
    /// Show the open card's whole body, or fold it back to its rows.
    ViewCard,
    /// Move the open card's body by this many rows; a negative count moves it
    /// up.
    ScrollCard(isize),
    /// Walk the mode cycle to its next member.
    CycleMode,
    /// Say this instead of doing anything: the state the press met refuses it.
    Refused(&'static str),
}

/// What `key` means to `model` now, or nothing at all.
pub(crate) fn intent(model: &Model, key: KeyEvent) -> Option<Intent> {
    if model.card().is_some() {
        return card_key(model, key);
    }
    composer_key(model, key)
}

/// The card's keys: its own answers (`cards_keys`), and a refusal of the one
/// global key that would otherwise act on a state the operator cannot see
/// behind the card. Enter is deliberately none of them, so a press meant for
/// the composer during a card can never consent.
fn card_key(model: &Model, key: KeyEvent) -> Option<Intent> {
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    match key.code {
        KeyCode::BackTab => Some(Intent::Refused(CARD_HOLDS_MODE)),
        KeyCode::Tab if shift => Some(Intent::Refused(CARD_HOLDS_MODE)),
        _ => match cards_keys::card_intent(model.card()?, key)? {
            CardIntent::Answer(answer) => Some(Intent::AnswerCard(answer)),
            CardIntent::View => Some(Intent::ViewCard),
            CardIntent::Scroll(rows) => Some(Intent::ScrollCard(rows)),
        },
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
        // Only a held line can be dropped; with nothing queued the press is
        // not a request at all.
        KeyCode::Char('g') if ctrl && model.queued().is_some() => Some(Intent::DropQueued),
        KeyCode::BackTab => Some(Intent::CycleMode),
        KeyCode::Tab if modifiers.contains(KeyModifiers::SHIFT) => Some(Intent::CycleMode),
        KeyCode::Esc if model.is_busy() => Some(Intent::CancelTurn),
        KeyCode::Backspace => Some(Intent::Backspace),
        KeyCode::Char(character) if !ctrl && !alt => Some(Intent::Insert(character)),
        _ => None,
    }
}
