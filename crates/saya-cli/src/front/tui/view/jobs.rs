//! The background jobs drawer (`02` §8): the jobs a run has started, the row
//! the operator has selected, and the three commands the list offers.
//!
//! The pane that paints those rows is [`super::jobs_rows`]. This module owns
//! only what the drawer *is*, so moving its selection, asking before a cancel
//! and reading a job's detail are decisions a test can make without a key
//! press. The keys that open it — `Ctrl+J` and bare `/job` — and the keymap
//! that walks it belong to the runtime.
//!
//! `x` cancels nothing. It puts the drawer in a confirm state that owns the
//! keyboard and names the row it would cancel; only a `y` answering it yields
//! the id to cancel, and `n` or Esc withdraws the ask (`02` §8, the same shape
//! T12's `d` takes). `a` approves nothing either: it opens the card for the
//! call the selected job is waiting on, so a job's call is never consented to
//! unseen (`02` §5).

use crate::front::tui::theme::{Charset, Glyph, glyph};

/// The pane's title, on its top border.
pub(crate) const TITLE: &str = "Background jobs";

/// What the drawer says when the run has no jobs at all.
pub(crate) const NO_JOBS: &str = "no jobs";

/// The verb the confirm asks with: `cancel <id>? y/n`.
pub(crate) const CONFIRM_ASK: &str = "cancel";

/// What `a` says when a call is already open: `02` §5's card is answered one at
/// a time, and a job's call queues behind it (T9/A5).
pub(crate) const CARD_FIRST: &str = "the open card comes first — answer it, then approve";

/// What `↵` says when the job's own transcript is not part of this surface yet:
/// the row's summary line is shown, and the drawer says what is missing.
pub(crate) const DETAIL_UNIT: &str =
    "job detail needs the job transcript, which this surface does not have yet — its last summary";

/// What a row says about a job whose call is waiting for the operator (`02` §5).
pub(crate) const WAITING_FOR_OPERATOR: &str = "waiting for your approval";

/// The state a job is in, as `02` §8's three rows name it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum State {
    /// `● running`.
    #[default]
    Running,
    /// `⏸ paused`.
    Paused,
    /// `✓ finished`.
    Finished,
}

impl State {
    /// The word the row says for this state.
    pub(crate) const fn word(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Finished => "finished",
        }
    }

    /// The mark the row leads with: `●`, `⏸`, `✓`.
    pub(crate) const fn mark(self) -> Glyph {
        match self {
            Self::Running => Glyph::Bullet,
            Self::Paused => Glyph::Paused,
            Self::Finished => Glyph::Check,
        }
    }
}

/// One job the drawer lists: the id it is named by, the state it is in, how
/// long it has run, how many helpers it has out, and the last thing it said.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Job {
    /// The job's id, as `/job show <id>` and the drawer name it.
    pub(crate) id: String,
    /// The state the job's plumbing reports.
    pub(crate) state: State,
    /// How long it has been running, as the job's own words state it (`3m`).
    pub(crate) age: String,
    /// How many helpers it has working for it.
    pub(crate) helpers: usize,
    /// The last thing it said: a paused job's reason, a finished job's result.
    pub(crate) summary: String,
    /// The call the operator owes this job, by the question's own id. `a` opens
    /// the card for it; the drawer never answers it itself.
    pub(crate) call: Option<u64>,
}

impl Job {
    /// The facts beside the row's mark: the state's word and what belongs to
    /// it — a running job's age and helpers, a paused job's reason, a finished
    /// job's last summary (`02` §8).
    pub(crate) fn facts(&self) -> String {
        match self.state {
            // `02` §8 draws a paused row's reason where a running row carries its
            // age, because the reason is what an operator acts on.
            State::Paused => format!("paused: {}", self.summary),
            State::Finished if self.summary.is_empty() => String::from("finished"),
            State::Finished => format!("finished · {}", self.summary),
            State::Running => {
                let age = match self.age.as_str() {
                    "" => String::new(),
                    age => format!(" {age}"),
                };
                let helpers = match self.helpers {
                    0 => String::new(),
                    1 => String::from(" · 1 helper"),
                    helpers => format!(" · {helpers} helpers"),
                };
                // A job whose call is on the card says what it waits for: that is
                // the one fact an operator can act on from the drawer (`02` §5).
                let waiting = if self.call.is_some() {
                    format!(" · {WAITING_FOR_OPERATOR}")
                } else {
                    String::new()
                };
                format!("running{age}{helpers}{waiting}")
            }
        }
    }
}

/// The drawer: the rows it lists, the row selected in them, and whether `x` is
/// waiting for an answer.
#[derive(Debug, Clone)]
pub(crate) struct Drawer {
    /// Every job the run has, oldest first.
    pub(crate) rows: Vec<Job>,
    /// The selected row's index, clamped to the rows that exist.
    pub(crate) selected: usize,
    /// Whether `x` asked and the operator has not answered: while it is set the
    /// confirm owns the keyboard, and only `y`, `n` and Esc are read.
    pub(crate) confirming: bool,
    /// Which glyph set the pane draws with.
    pub(crate) charset: Charset,
}

impl Default for Drawer {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            selected: 0,
            confirming: false,
            charset: Charset::Utf8,
        }
    }
}

impl Drawer {
    /// The drawer over `rows`, the first one selected.
    pub(crate) fn new(rows: Vec<Job>) -> Self {
        Self {
            rows,
            ..Self::default()
        }
    }

    /// Whether the drawer lists no job at all.
    pub(crate) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The selected row's index, clamped to the rows there are.
    pub(crate) fn selected(&self) -> usize {
        self.selected.min(self.rows.len().saturating_sub(1))
    }

    /// The selected row, when there is one.
    pub(crate) fn selected_row(&self) -> Option<&Job> {
        self.rows.get(self.selected())
    }

    /// Whether `x` is waiting for an answer.
    pub(crate) const fn is_confirming(&self) -> bool {
        self.confirming
    }

    /// What the confirm asks, when it is asking: `cancel <id>? y/n`.
    pub(crate) fn ask(&self) -> Option<String> {
        if !self.confirming {
            return None;
        }
        let row = self.selected_row()?;
        Some(format!("{CONFIRM_ASK} {}? y/n", row.id))
    }

    /// Moves the selection one row down, and no further than the last row.
    pub(crate) fn down(&mut self) {
        if self.confirming || self.rows.is_empty() {
            return;
        }
        self.selected = (self.selected() + 1).min(self.rows.len() - 1);
    }

    /// Moves the selection one row up, and no further than the first row.
    pub(crate) fn up(&mut self) {
        if self.confirming || self.rows.is_empty() {
            return;
        }
        self.selected = self.selected().saturating_sub(1);
    }

    /// Asks before cancelling the selected row. This press cancels nothing: it
    /// puts the drawer in the confirm state, which is the only way to a cancel.
    pub(crate) fn ask_cancel(&mut self) {
        if !self.confirming && self.selected_row().is_some() {
            self.confirming = true;
        }
    }

    /// Answers the confirm: `y` yields the id to cancel, `n` withdraws the ask.
    /// The drawer stays up either way — the runtime takes the cancelled job out
    /// of it once its plumbing says so.
    pub(crate) fn answer(&mut self, yes: bool) -> Option<String> {
        if !self.confirming {
            return None;
        }
        self.confirming = false;
        if !yes {
            return None;
        }
        Some(self.selected_row()?.id.clone())
    }

    /// Esc: withdraws the ask when the confirm is open, and closes the drawer
    /// when it is not. Neither asks the runtime for anything.
    pub(crate) fn escape(&mut self) {
        self.confirming = false;
    }

    /// `↵`: the selected job's detail. A job's own turn transcript is not part
    /// of this surface yet, so the detail is the job's summary line, and the
    /// drawer says which unit is missing rather than showing nothing.
    pub(crate) fn detail(&self) -> Option<Vec<String>> {
        let row = self.selected_row()?;
        Some(vec![self.line(row), String::from(DETAIL_UNIT)])
    }

    /// The row's own line: its mark, its id, and the facts beside them.
    pub(crate) fn line(&self, row: &Job) -> String {
        format!(
            "{} {}  {}",
            glyph(row.state.mark(), self.charset),
            row.id,
            row.facts()
        )
    }
}
