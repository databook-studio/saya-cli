//! The card's keys: one press read as the answer the operator means, and never
//! as consent they did not give.
//!
//! Today's answers stay today's (`interactive/tui/keys/approvals.rs`): `y`/`a`
//! allow once, `s` grants exactly the token the card offered, `n`/`d`/Esc deny.
//! **Enter is deliberately none of them** — the card can appear while the
//! operator is typing, and a press meant for the composer must never answer a
//! call. A held Ctrl, Alt or Super is not an answer either: `Ctrl+A` or `Ctrl+D`
//! mid-word must never read as consent or denial, whatever the card offered.
//!
//! `v` opens the body the six-row cap withheld and folds it back, and the
//! arrows and page keys move it; a scroll key is never an answer, so the keys
//! that reach the withheld facts can never be the keys that consent.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::agent::tools::consent::Answer;
use crate::chat::Question;

/// How far one page key moves the card's body, in rows.
pub(crate) const CARD_PAGE_ROWS: isize = 6;

/// What one press means to the open card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CardIntent {
    /// Answer the call the card is showing with this.
    Answer(Answer),
    /// Show the whole body, or fold it back to the card's rows.
    View,
    /// Move the body by this many rows; a negative count moves it up.
    Scroll(isize),
}

/// What `key` means to the call `question` puts: an answer, a look at the
/// withheld body, a scroll, or nothing at all.
pub(crate) fn card_intent(question: &Question, key: KeyEvent) -> Option<CardIntent> {
    if held(key.modifiers) {
        return None;
    }
    match key.code {
        KeyCode::Up => Some(CardIntent::Scroll(-1)),
        KeyCode::Down => Some(CardIntent::Scroll(1)),
        KeyCode::PageUp => Some(CardIntent::Scroll(-CARD_PAGE_ROWS)),
        KeyCode::PageDown => Some(CardIntent::Scroll(CARD_PAGE_ROWS)),
        KeyCode::Char('y' | 'Y' | 'a' | 'A') => Some(CardIntent::Answer(Answer::Once)),
        // A session grant needs a token to record: a call the operator was
        // offered none for has none to take, so `s` is no answer at all.
        KeyCode::Char('s' | 'S') => question
            .grant
            .as_ref()
            .map(|_| CardIntent::Answer(Answer::Session)),
        KeyCode::Char('n' | 'N' | 'd' | 'D') | KeyCode::Esc => {
            Some(CardIntent::Answer(Answer::Deny))
        }
        KeyCode::Char('v' | 'V') => Some(CardIntent::View),
        _ => None,
    }
}

/// A held Ctrl, Alt or Super makes the press an editing or global chord, never
/// an answer and never a scroll. Shift is not held in this sense: `Y`/`N` are
/// the answers their bare forms are.
fn held(modifiers: KeyModifiers) -> bool {
    modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
}
