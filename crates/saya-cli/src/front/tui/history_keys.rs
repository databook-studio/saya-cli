//! The input history's keys: `Up`/`Down` walking the ring, `Ctrl+R` searching
//! it, and the search's own keys while it is open (`02` §7).
//!
//! The history is the composer's, not a pane's: it is read here only while no
//! card and no overlay owns the keyboard, and the draft stays on the line
//! throughout — a recall replaces it (and stashes it, so Down brings it back),
//! and a cancelled search leaves it untouched.
//!
//! While a search is open it owns the keys, exactly as a pane does: a typed
//! character narrows the query instead of reaching the composer behind it, so
//! what the operator is typing is always what the search is looking for.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::keys::Intent;
use super::model::Model;

/// What one history key asks the model to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HistoryIntent {
    /// Recall the previous ring entry into the composer (`Up`).
    Previous,
    /// Recall the next one, and the stashed draft past the newest (`Down`).
    Next,
    /// Open the reverse search (`Ctrl+R`).
    Search,
    /// Add this character to the open search's query.
    SearchType(char),
    /// Remove the open search's last query character.
    SearchBackspace,
    /// Step the open search one match older (a second `Ctrl+R`).
    SearchOlder,
    /// Take the entry the search shows onto the line (`Enter`).
    SearchAccept,
    /// Leave the line as it was (`Esc`).
    SearchClose,
}

/// The keys the open search reads: a character narrows it, `Ctrl+R` steps one
/// match older, `Enter` takes the match and Esc closes it. The character arm is
/// last, so no control chord can be typed into the query.
pub(crate) fn search_key(key: KeyEvent) -> Option<Intent> {
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let action = match key.code {
        KeyCode::Char('r') if control => HistoryIntent::SearchOlder,
        KeyCode::Enter if !alt => HistoryIntent::SearchAccept,
        KeyCode::Esc => HistoryIntent::SearchClose,
        KeyCode::Backspace => HistoryIntent::SearchBackspace,
        KeyCode::Char(character) if !control && !alt => HistoryIntent::SearchType(character),
        _ => return None,
    };
    Some(Intent::History(action))
}

/// Carries one history intent out. Nothing here leaves the surface — a recall
/// fills the line and a search only reads — so no command is answered with.
pub(crate) fn press(model: &mut Model, action: HistoryIntent) {
    match action {
        HistoryIntent::Previous => model.recall_previous(),
        HistoryIntent::Next => model.recall_next(),
        HistoryIntent::Search => model.open_search(),
        HistoryIntent::SearchType(character) => model.search_type(character),
        HistoryIntent::SearchBackspace => model.search_backspace(),
        HistoryIntent::SearchOlder => model.search_older(),
        HistoryIntent::SearchAccept => model.accept_search(),
        HistoryIntent::SearchClose => model.close_search(),
    }
}
