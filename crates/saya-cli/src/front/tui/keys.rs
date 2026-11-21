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
use super::direct_trust;
use super::model::Model;
use super::model_overlay::Overlay;
use crate::agent::tools::consent::Answer;
use crate::interactive::tui::TrustResolution;

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
    /// Move the mode picker's selection one row up.
    ModeUp,
    /// Move the mode picker's selection one row down.
    ModeDown,
    /// State the mode the picker's selected row names.
    ModeSet,
    /// Close the picker without stating anything.
    ModeCancel,
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
        return overlay_key(model, overlay, key);
    }
    if model.card().is_some() {
        return card_key(model, key);
    }
    if model.in_cursor_mode() {
        return cursor_key(key);
    }
    composer_key(model, key)
}

/// The open pane's keys: while one is up it owns the keyboard, so no press can
/// reach the composer behind it.
fn overlay_key(model: &Model, overlay: &Overlay, key: KeyEvent) -> Option<Intent> {
    match overlay {
        Overlay::Mode(_) => mode_key(key),
        Overlay::Jobs => jobs_key(model.jobs().is_confirming(), key),
        Overlay::Trust => trust_key(key),
    }
}

/// The jobs drawer's keys (`02` §8): `↑↓` walk its rows, `↵` opens the selected
/// job, `a` opens the card for the call it is waiting on, `x` asks before it
/// cancels, and Esc closes the drawer. While the confirm asks, only `y`, `n` and
/// Esc are read — the ask owns the keyboard, exactly as T12's `d` does.
fn jobs_key(confirming: bool, key: KeyEvent) -> Option<Intent> {
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

/// The workspace-trust question's keys (`02` §9.4): `y` binds the launch folder
/// and `n` continues without one — the two answers today's modal gives through
/// `t` and `c`, read through the same resolution (`direct_trust::press`).
fn trust_key(key: KeyEvent) -> Option<Intent> {
    match direct_trust::press(key.code)? {
        TrustResolution::TrustCwd => Some(Intent::TrustAccept),
        _ => Some(Intent::TrustDecline),
    }
}

/// `/mode`'s picker: ↑↓ choose, Enter set, Esc cancel (`02` §8).
fn mode_key(key: KeyEvent) -> Option<Intent> {
    match key.code {
        KeyCode::Up => Some(Intent::ModeUp),
        KeyCode::Down => Some(Intent::ModeDown),
        KeyCode::Enter if !key.modifiers.contains(KeyModifiers::ALT) => Some(Intent::ModeSet),
        KeyCode::Esc => Some(Intent::ModeCancel),
        _ => None,
    }
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
        KeyCode::Esc if model.is_busy() => Some(Intent::CancelTurn),
        KeyCode::Backspace => Some(Intent::Backspace),
        KeyCode::Char(character) if !ctrl && !alt => Some(Intent::Insert(character)),
        _ => None,
    }
}
