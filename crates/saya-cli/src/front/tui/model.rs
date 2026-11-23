//! The new TUI's model: everything the surface owns between two messages.
//!
//! One owner of state, as the Elm shape of the front needs: the conversation
//! the view model folds, the composer's text, the call the operator has to
//! answer, the mode in force and the one message a running turn holds behind
//! it. Plain data — no terminal, no widget, and a clock it is handed rather
//! than read — so a test can drive a whole session by hand ([`super::update`]).

use std::time::Instant;

use saya_agent::PermissionMode;

use super::focus::EntryPath;
use super::model_card::Seat;
use super::model_history::Search;
use super::model_overlay::Overlay;
use super::view::jobs::Drawer;
use crate::chat::Question;
use crate::front::tui::view::Toast;
use crate::front::{TranscriptView, TurnView};
use crate::interactive::tui::History;

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
    /// The calls that arrived while a card was open, oldest first: they take the
    /// card in that order as it is answered (T9/A5 — a job's ask queues behind
    /// the session's and never replaces it).
    pub(crate) held: Vec<Question>,
    /// The run's jobs, as the job plumbing reports them (`02` §8): the drawer's
    /// rows, kept current whether or not the drawer is open.
    pub(crate) jobs: Drawer,
    /// The answer the operator's arrows point at while a card is open: `None`
    /// until an arrow press lands on the first answer, and `None` again as soon
    /// as a character is typed — the composer's Enter must never become a
    /// consent by accident.
    pub(crate) card_choice: Option<usize>,
    /// Which seat holds the keyboard while a card is open (UI3): the panel
    /// answers only when it has it, and typing is never an answer.
    pub(crate) seat: Seat,
    /// The injected clock's last reading: taken at the message being folded,
    /// never read here (a test turns it by hand).
    pub(crate) now: Option<Instant>,
    /// The reading the last key press was folded at, so a call that opens while
    /// the operator is typing leaves the keyboard where their hands are.
    pub(crate) typed_at: Option<Instant>,
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
    /// The composer's submitted lines, as today's surface keeps them: the ring
    /// `Up`/`Down` walk, persisted to the `input_history` file, redacting
    /// `--param` values on the way in (`02` §7).
    pub(crate) history: History,
    /// The reverse search `Ctrl+R` opens over the ring, while it is open.
    pub(crate) search: Option<Search>,
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
    /// The entry the block cursor holds (`02` §6), if it holds one. While it
    /// does, the keyboard is the cursor's: Esc leaves it, Enter expands the
    /// block and `←` folds it, `c` copies it and `i` shows its provenance.
    pub(crate) focus: Option<EntryPath>,
    /// Whether the focused entry shows its inspector line (`02` §6's `i`).
    pub(crate) inspector: bool,
    /// Whether the transcript shows every step's full command and the outcome
    /// it reported (`Ctrl+O`), or one compact line per step. The compact
    /// transcript is the default: the steps are visible without a key.
    pub(crate) detail: bool,
    /// The frame a running step's spinner is drawn from: one step per tick, so
    /// a step in flight reads as working rather than as a settled dot.
    pub(crate) spinner: usize,
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

    /// Puts a call to the operator: one card at a time, opening folded at its
    /// first row, with the keyboard where the operator's hands are
    /// ([`Model::opening_seat`]).
    pub(crate) fn show_card(&mut self, card: Question) {
        self.card = Some(card);
        self.card_full = false;
        self.card_scroll = 0;
        self.card_choice = None;
        self.seat = self.opening_seat();
    }

    /// Takes the open card, if one is waiting for an answer.
    pub(crate) fn take_card(&mut self) -> Option<Question> {
        self.card_full = false;
        self.card_scroll = 0;
        self.card_choice = None;
        self.seat = Seat::Composer;
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

    /// One tick of the surface's clock: the toast ages, and the spinner turns
    /// one frame, so a step in flight moves on a screen nothing else moves.
    pub(crate) fn tick(&mut self) {
        self.spinner = self.spinner.wrapping_add(1);
        self.tick_toast();
    }
}
