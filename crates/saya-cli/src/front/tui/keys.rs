//! The keymap: what one key press means to the model, in the state it is in.
//!
//! A press is read here and nowhere else, so the same key can mean different
//! things without the update growing a decision tree: Esc answers an open card
//! and otherwise stops a running turn, and Shift+Tab is refused outright while
//! a card waits — a mode switch the operator asked for mid-answer would leave
//! the question they were reading behind.
//!
//! The card owns the keyboard while it is open, and its own answers are read
//! there ([`super::cards_keys`]); the composer owns it otherwise. A pane that
//! owns the screen reads its keys in [`super::overlay_keys`].
//!
//! Two of the composer's own keys open a pane rather than type: `Ctrl+K` opens
//! the command palette, and `?` — like `/` — opens it only while the composer is
//! empty, so a draft keeps the character it asked for (`02` §11).

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::cards_keys::{self, CardIntent};
use super::history_keys::{self, HistoryIntent};
use super::model::Model;
use super::overlay_keys;
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
    /// Walk the answers the open card offers, a negative count moving up.
    FocusChoice(isize),
    /// Give the answer an arrow press pointed at, and nothing at all when none
    /// is pointed at.
    AnswerFocused,
    /// Walk the mode cycle to its next member.
    CycleMode,
    /// Move the mode picker's selection one row up.
    ModeUp,
    /// Move the mode picker's selection one row down.
    ModeDown,
    /// State the mode the picker's selected row names.
    ModeSet,
    /// Close the picker without stating anything.
    ModeCancel,
    /// Open `02` §11's command palette (`Ctrl+K`, or `/` on an empty composer).
    OpenPalette,
    /// Open `02` §11's key sheet (`?` on an empty composer).
    OpenHelp,
    /// Add this character to the palette's query.
    PaletteInsert(char),
    /// Remove the palette's last query character.
    PaletteBackspace,
    /// Move the palette's selection one row up.
    PaletteUp,
    /// Move the palette's selection one row down.
    PaletteDown,
    /// Run the command the palette's selected row names.
    PaletteRun,
    /// Close the palette without running anything.
    PaletteClose,
    /// Close the key sheet.
    HelpClose,
    /// Walk, search or leave the composer's input history (`02` §7): `Up`,
    /// `Down`, `Ctrl+R` and the keys the open search reads.
    History(history_keys::HistoryIntent),
    /// Walk the block cursor one entry older (`02` §6).
    FocusUp,
    /// Walk the block cursor one entry newer.
    FocusDown,
    /// Expand the block the cursor holds, or fold it back.
    FocusExpand,
    /// Fold the block the cursor holds.
    FocusFold,
    /// Copy the focused entry's source text.
    FocusCopy,
    /// Show the focused entry's inspector line, or take it away.
    FocusInspector,
    /// Leave cursor mode.
    FocusLeave,
    /// Answer the workspace-trust question by binding the launch folder
    /// (`02` §9.4's `y`).
    TrustAccept,
    /// Answer it by continuing without a workspace (`02` §9.4's `n`).
    TrustDecline,
    /// Say this instead of doing anything: the state the press met refuses it.
    Refused(&'static str),
    /// Open the background jobs drawer (`02` §8's `Ctrl+J`).
    OpenJobs,
    /// Move the drawer's selection one row up.
    JobsUp,
    /// Move the drawer's selection one row down.
    JobsDown,
    /// Show the selected job's detail.
    JobsOpen,
    /// Approve the selected job's call through the card it belongs on.
    JobsApprove,
    /// Ask before cancelling the selected job (`02` §8's `x`).
    JobsCancelAsk,
    /// Answer the confirm: the selected job is cancelled.
    JobsCancelYes,
    /// Answer the confirm: nothing is cancelled.
    JobsCancelNo,
    /// Close the drawer without stating anything.
    JobsClose,
}

/// What `key` means to `model` now, or nothing at all.
pub(crate) fn intent(model: &Model, key: KeyEvent) -> Option<Intent> {
    if let Some(overlay) = model.overlay() {
        return overlay_keys::key(model, overlay, key);
    }
    if model.card().is_some() {
        return card_key(model, key);
    }
    if model.in_cursor_mode() {
        return cursor_key(key);
    }
    // The open search owns the keys, exactly as a pane does: what the operator
    // types is what it is looking for, never text on the line behind it.
    if model.search().is_some() {
        return history_keys::search_key(key);
    }
    composer_key(model, key)
}

/// The card's keys: its own answers (`cards_keys`), and a refusal of the one
/// global key that would otherwise act on a state the operator cannot see
/// behind the card. Enter is deliberately none of them, so a press meant for
/// the composer during a card can never consent.
///
/// Every other press is the composer's, exactly as it was before the card
/// arrived: a call can land mid-sentence, and the line the operator was typing
/// — `/job approve <id>` among them, which is how a job's consent is given by
/// name (`02` §8) — must still be typeable and still be sent by Enter.
fn card_key(model: &Model, key: KeyEvent) -> Option<Intent> {
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    match key.code {
        // Enter is the composer's unless the operator has pointed at an answer
        // with the arrows: only then is it one of the card's own keys, and the
        // press cannot be a draft's.
        KeyCode::Enter if model.card_choice().is_some() => Some(Intent::AnswerFocused),
        KeyCode::BackTab => Some(Intent::Refused(CARD_HOLDS_MODE)),
        KeyCode::Tab if shift => Some(Intent::Refused(CARD_HOLDS_MODE)),
        _ => match cards_keys::card_intent(model.card()?, key) {
            Some(CardIntent::Answer(answer)) => Some(Intent::AnswerCard(answer)),
            Some(CardIntent::View) => Some(Intent::ViewCard),
            Some(CardIntent::Scroll(rows)) => Some(Intent::ScrollCard(rows)),
            Some(CardIntent::Focus(step)) => Some(Intent::FocusChoice(step)),
            None => composer_fallback(model, key),
        },
    }
}

/// A press the card does not answer, read as the composer reads it — and only
/// as far as the line goes. `/` and `?` would open a pane over the call the
/// operator is being asked about, so they are the characters they were before
/// a pane existed; a pane's chord and every other key that would act on a
/// state the card is covering are not requests while it is open.
fn composer_fallback(model: &Model, key: KeyEvent) -> Option<Intent> {
    match composer_key(model, key) {
        Some(Intent::OpenPalette | Intent::OpenHelp) => match key.code {
            KeyCode::Char(character) => Some(Intent::Insert(character)),
            _ => None,
        },
        intent @ (Some(Intent::Insert(_)) | Some(Intent::Newline) | Some(Intent::Backspace))
        | intent @ Some(Intent::Submit) => intent,
        _ => None,
    }
}

/// The block cursor's keys (`02` §6): while it holds an entry it owns the
/// keyboard, so no press reaches the composer behind it. `Enter`/`→` expand the
/// block, `←` folds it, `c` copies it, `i` shows its provenance, and Esc leaves
/// cursor mode. `e` is deliberately none of them: exporting a result needs the
/// export unit (T16), so this release leaves it unbound rather than half-wired.
fn cursor_key(key: KeyEvent) -> Option<Intent> {
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Up if alt => Some(Intent::FocusUp),
        KeyCode::Down if alt => Some(Intent::FocusDown),
        KeyCode::Enter if !alt => Some(Intent::FocusExpand),
        KeyCode::Right => Some(Intent::FocusExpand),
        KeyCode::Left => Some(Intent::FocusFold),
        KeyCode::Char('c') if !ctrl && !alt => Some(Intent::FocusCopy),
        KeyCode::Char('i') if !ctrl && !alt => Some(Intent::FocusInspector),
        KeyCode::Esc => Some(Intent::FocusLeave),
        _ => None,
    }
}

/// The composer's keys.
fn composer_key(model: &Model, key: KeyEvent) -> Option<Intent> {
    let modifiers = key.modifiers;
    let alt = modifiers.contains(KeyModifiers::ALT);
    let ctrl = modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        // The cursor is how a block is reached without a mouse: Alt+↑/↓ from the
        // composer takes it, and from no seat it lands on the newest entry.
        KeyCode::Up if alt => Some(Intent::FocusUp),
        KeyCode::Down if alt => Some(Intent::FocusDown),
        // The input history is the composer's own (`02` §7): `↑`/`↓` walk the
        // ring today's surface keeps, and `Ctrl+R` searches it.
        KeyCode::Up => Some(Intent::History(HistoryIntent::Previous)),
        KeyCode::Down => Some(Intent::History(HistoryIntent::Next)),
        KeyCode::Char('r') if ctrl => Some(Intent::History(HistoryIntent::Search)),
        KeyCode::Enter if alt => Some(Intent::Newline),
        KeyCode::Enter if !ctrl => Some(Intent::Submit),
        // Only a held line can be dropped; with nothing queued the press is
        // not a request at all.
        KeyCode::Char('g') if ctrl && model.queued().is_some() => Some(Intent::DropQueued),
        KeyCode::BackTab => Some(Intent::CycleMode),
        KeyCode::Tab if modifiers.contains(KeyModifiers::SHIFT) => Some(Intent::CycleMode),
        // The jobs drawer has a chord of its own, so no word of a message has to
        // give up its `j` (`02` §8).
        KeyCode::Char('j') if ctrl => Some(Intent::OpenJobs),
        // `02` §11's two panes: the palette has a chord, and both it and the key
        // sheet open from `/` and `?` only while the composer holds nothing — a
        // draft keeps the character it asked for.
        KeyCode::Char('k') if ctrl => Some(Intent::OpenPalette),
        KeyCode::Char('/') if !ctrl && !alt && model.composer().is_empty() => {
            Some(Intent::OpenPalette)
        }
        KeyCode::Char('?') if !ctrl && !alt && model.composer().is_empty() => {
            Some(Intent::OpenHelp)
        }
        KeyCode::Esc if model.is_busy() => Some(Intent::CancelTurn),
        KeyCode::Backspace => Some(Intent::Backspace),
        KeyCode::Char(character) if !ctrl && !alt => Some(Intent::Insert(character)),
        _ => None,
    }
}
