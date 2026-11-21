//! The new TUI's model: everything the surface owns between two messages.
//!
//! One owner of state, as the Elm shape of the front needs: the conversation
//! the view model folds, the composer's text, the call the operator has to
//! answer, the mode in force and the one message a running turn holds behind
//! it. Plain data — no terminal, no widget, no clock — so a test can drive a
//! whole session by hand ([`super::update`]).

use saya_agent::PermissionMode;

use super::model_overlay::Overlay;
use crate::chat::Question;
use crate::front::tui::view::Toast;
use crate::front::{TranscriptView, TurnView};

/// The surface's whole state between two messages.
#[derive(Debug, Default)]
pub(crate) struct Model {
    /// The conversation, as the turn view model holds it.
    pub(crate) transcript: TranscriptView,
    /// The composer's text: what Enter sends, and the only thing typing moves.
    pub(crate) composer: String,
    /// The call the operator has been asked and has not answered. One card at
    /// a time: the operator serialises its asks, so a second waits its turn.
    pub(crate) card: Option<Question>,
    /// The mode the run's next call reads, and whether this process allows
    /// `bypass` at all — the key cycle skips what the configuration withholds.
    pub(crate) mode: PermissionMode,
    pub(crate) allow_bypass: bool,
    /// Whether a turn is running.
    pub(crate) busy: bool,
    /// The database the run is connected to, if one is: the composer's idle
    /// placeholder asks about it by name (`02` §7, §9.3).
    pub(crate) database: Option<String>,
    /// Whether the run is paused at a stop the operator can continue from.
    pub(crate) paused: bool,
    /// Whether this surface is a read-only record of an older (0.5.0) session:
    /// the transcript is shown, and the next message starts a new run.
    pub(crate) read_only_record: bool,
    /// The one message a running turn holds behind it. One deep: the
    /// operator's last word wins, and a message it displaces is reported.
    pub(crate) queued: Option<String>,
    /// The last thing the surface said on its own — a refused key, a worker's
    /// word — until it expires or the next one replaces it. It lives on the
    /// tick ([`Toast`]): `02` §4 gives it four seconds, and a tick is the only
    /// clock the model has.
    pub(crate) toast: Option<Toast>,
    /// Whether the open card shows its whole body (`v`), and how far that body
    /// is scrolled. Both are the card's own reading of a press, and both reset
    /// with it: a new call opens folded, at its first row.
    pub(crate) card_full: bool,
    pub(crate) card_scroll: usize,
    /// The pane that owns the screen while it is open, if one does: it takes
    /// the keyboard before the card and the composer, and it is painted over
    /// the transcript (`02` §8).
    pub(crate) overlay: Option<Overlay>,
}

impl Model {
    /// The conversation, oldest turn first.
    pub(crate) fn turns(&self) -> &[TurnView] {
        self.transcript.turns()
    }

    /// The composer's text, as typing left it.
    pub(crate) fn composer(&self) -> &str {
        &self.composer
    }

    /// The message a running turn holds behind it, if any.
    pub(crate) fn queued(&self) -> Option<&str> {
        self.queued.as_deref()
    }

    /// The call waiting for an answer, if one does.
    pub(crate) fn card(&self) -> Option<&Question> {
        self.card.as_ref()
    }

    /// The last thing the surface said on its own, if it still says it.
    pub(crate) fn toast(&self) -> Option<&str> {
        self.toast.as_ref().map(Toast::text)
    }

    /// The last thing the surface said, with the ticks it has left.
    pub(crate) fn toast_value(&self) -> Option<&Toast> {
        self.toast.as_ref()
    }

    /// Whether the open card's whole body is shown, or only the rows the
    /// screen has for it.
    pub(crate) const fn card_full(&self) -> bool {
        self.card_full
    }

    /// How far the open card's body is scrolled, in rows.
    pub(crate) const fn card_scroll(&self) -> usize {
        self.card_scroll
    }

    /// The mode in force, as the status line's chip names it.
    pub(crate) const fn mode(&self) -> PermissionMode {
        self.mode
    }

    /// Whether a turn is running.
    pub(crate) const fn is_busy(&self) -> bool {
        self.busy
    }

    /// The database the run is connected to, named as the status line and the
    /// composer name it.
    pub(crate) fn database_name(&self) -> Option<&str> {
        self.database.as_deref()
    }

    /// Whether the run is paused at a stop the operator can continue from.
    pub(crate) const fn is_paused(&self) -> bool {
        self.paused
    }

    /// Whether this surface is a read-only record of an older session.
    pub(crate) const fn is_read_only_record(&self) -> bool {
        self.read_only_record
    }

    /// Puts one typed character at the composer's end.
    pub(crate) fn push(&mut self, character: char) {
        self.composer.push(character);
    }

    /// Puts a pasted block at the composer's end, verbatim: a paste is text,
    /// never a key press.
    pub(crate) fn insert(&mut self, text: &str) {
        self.composer.push_str(text);
    }

    /// Removes the composer's last character, if it has one.
    pub(crate) fn backspace(&mut self) -> bool {
        self.composer.pop().is_some()
    }

    /// Takes the composer's text, leaving the composer empty.
    pub(crate) fn take_composer(&mut self) -> String {
        std::mem::take(&mut self.composer)
    }

    /// Holds `text` behind the running turn, and answers the line it
    /// displaced: the queue is one deep, so the operator's last word wins.
    pub(crate) fn queue(&mut self, text: String) -> Option<String> {
        self.queued.replace(text)
    }

    /// Takes the held line, if there is one.
    pub(crate) fn take_queued(&mut self) -> Option<String> {
        self.queued.take()
    }

    /// Drops the line held behind the running turn, if there is one: Ctrl+G's
    /// whole effect (`02` §7). Nothing is sent, and the turn keeps running.
    pub(crate) fn drop_queued(&mut self) {
        self.queued = None;
    }

    /// Opens a turn with the operator's own words.
    pub(crate) fn begin_turn(&mut self, message: String) {
        self.transcript.begin_turn(message);
    }

    /// Marks the running turn as started.
    pub(crate) fn start_turn(&mut self) {
        self.busy = true;
    }

    /// Marks the running turn as ended.
    pub(crate) fn end_turn(&mut self) {
        self.busy = false;
    }

    /// Puts a call to the operator: one card at a time, opening folded at its
    /// first row.
    pub(crate) fn show_card(&mut self, card: Question) {
        self.card = Some(card);
        self.card_full = false;
        self.card_scroll = 0;
    }

    /// Takes the open card, if one is waiting for an answer.
    pub(crate) fn take_card(&mut self) -> Option<Question> {
        self.card_full = false;
        self.card_scroll = 0;
        self.card.take()
    }

    /// Shows the open card's whole body, or folds it back to its rows.
    pub(crate) fn toggle_card_full(&mut self) {
        self.card_full = !self.card_full;
    }

    /// Moves the open card's body by `rows`, a negative count moving it up. It
    /// never moves above its first row; the pane clamps the far end against
    /// the body it paints.
    pub(crate) fn scroll_card(&mut self, rows: isize) {
        let moved = self.card_scroll as isize + rows;
        self.card_scroll = moved.max(0) as usize;
    }

    /// States the mode the run's next call reads.
    pub(crate) fn set_mode(&mut self, mode: PermissionMode) {
        self.mode = mode;
    }

    /// Says something in the surface's own voice, replacing the last thing it
    /// said and restarting its clock: the newer words get the whole four
    /// seconds (`02` §4).
    pub(crate) fn say(&mut self, message: impl Into<String>) {
        self.toast = Some(Toast::new(message));
    }

    /// One tick of the toast's clock. A toast spends `02` §4's four seconds of
    /// ticks and is then gone; with nothing said, the tick changes nothing.
    pub(crate) fn tick_toast(&mut self) {
        if let Some(toast) = &mut self.toast
            && !toast.tick()
        {
            self.toast = None;
        }
    }
}
