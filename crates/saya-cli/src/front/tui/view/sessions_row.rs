//! One row of the sessions picker: the facts it carries, and what resuming it
//! would do — the state the pane draws as a mark, and whether it opens
//! read-only.
//!
//! A row is built from three sources and nothing else: the store's own listing
//! row (its id, and the stamp its age is measured from), the record's words (its
//! title, the database it was on, how many turns it settled), and the resume
//! decision the chat layer already makes. The decision is what the row's state
//! *is*, so the pane cannot disagree with the resume that follows it.

use crate::chat::Resume;
use crate::commands::saved_sessions::SessionRow as StoredRow;
use crate::interactive::tui::replay::relative_time;

/// The three states a listed session can be in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum State {
    /// The run is open in this process: the session this surface is on.
    Open,
    /// The run finished. Resuming starts a fresh run seeded with the
    /// conversation, so the record is not read-only.
    Finished,
    /// The record carries no run this process can attach to or re-prepare, so
    /// it opens read-only until the next message prepares one.
    Record,
}

impl State {
    /// The state a resume decision puts the row in.
    ///
    /// A refusal is a record too: a job this release takes no further turn on
    /// has no run to attach to or to re-prepare, so it opens read-only and the
    /// pane says so rather than offering a resume that would be refused.
    pub(crate) fn of(decision: &Resume) -> Self {
        match decision {
            Resume::Attach { .. } => Self::Open,
            Resume::PrepareNewEpoch { .. } => Self::Finished,
            Resume::ReadOnly { .. } | Resume::Refused { .. } => Self::Record,
        }
    }

    /// Whether a row in this state opens read-only.
    pub(crate) const fn read_only(self) -> bool {
        matches!(self, Self::Record)
    }
}

/// The words a record adds to the store's own listing row: what an operator
/// recognises the session by, and how long it ran.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Facts {
    /// The session's title: the conversation's own opening words.
    pub(crate) title: String,
    /// The database the session was on, when it was on one.
    pub(crate) database: Option<String>,
    /// How many turns the record settled.
    pub(crate) turns: usize,
}

/// One row of the picker: the facts an operator chooses by, and what resuming
/// it would do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionRow {
    /// The store's id: what a resume or a delete names.
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) database: Option<String>,
    pub(crate) turns: usize,
    /// How long ago the record was written, in the words the listing uses.
    pub(crate) age: String,
    pub(crate) state: State,
    /// Whether the session opens read-only, which is what [`State::Record`]
    /// means. Carried as a fact so a caller reads the row rather than the enum.
    pub(crate) read_only: bool,
}

impl SessionRow {
    /// The row a picker lists: the store's row, the record's words and the
    /// decision resuming it would take.
    pub(crate) fn listed(
        stored: &StoredRow,
        facts: Facts,
        decision: &Resume,
        now_ms: u128,
    ) -> Self {
        let state = State::of(decision);
        Self {
            id: stored.id.clone(),
            title: facts.title,
            database: facts.database,
            turns: facts.turns,
            age: relative_time(now_ms.saturating_sub(stored.modified_unix_ms)),
            state,
            read_only: state.read_only(),
        }
    }

    /// Whether this is the session the surface is on: the run open in this
    /// process, which the pane tags `(this session)`.
    pub(crate) const fn current(&self) -> bool {
        matches!(self.state, State::Open)
    }
}
