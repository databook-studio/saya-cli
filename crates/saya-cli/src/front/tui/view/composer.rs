//! The composer box: the draft, the one line held behind a running turn, and
//! the words the current state puts around them (`02` §7).
//!
//! The box is drawn from the model and nothing else: its title, its
//! placeholder and its hint row are the state table of `02` §7, and the frame
//! around them wraps the draft with today's input buffer rather than with a
//! second wrapping rule ([`super::composer_frame`]). Every run carries the
//! [`Token`] it is painted with, so a pane above this one resolves light, dark
//! and no-colour without knowing what the text means.
//!
//! `02` §7's queue is one deep: a held line is shown dim, under the title
//! `Queued`, with `↑ edit queued · Ctrl+G drop` for its hint. The drop key
//! belongs to the keymap and the update; this module only draws the result.

use crate::front::tui::model::Model;
use crate::front::tui::theme::Token;
use crate::interactive::tui::{Candidate, slash_candidates};

/// The six states of `02` §7's table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum State {
    /// A database is connected and nothing is running.
    IdleWithDatabase,
    /// No database is connected and nothing is running.
    IdleWithoutDatabase,
    /// A turn is running; Enter queues behind it.
    TurnRunning,
    /// A turn is running and one line is held behind it.
    OneQueued,
    /// The run is paused at a stop the operator can continue from.
    Paused,
    /// A read-only record of an older session; the next message starts a run.
    ReadOnlyRecord,
    /// A call is waiting for an answer: the composer cannot be typed into.
    Disabled,
}

/// The queued state's hint row, and the keys it names (`02` §7).
pub(crate) const QUEUED_HINT: &str = "↑ edit queued · Ctrl+G drop";

/// The idle hint row at the full layout (`02` §7).
pub(crate) const IDLE_HINT: &str = "Enter send · Alt+Enter newline · Ctrl+K commands";

/// The idle hint row at the compact layout: the middle segment is the one
/// that drops, because `Ctrl+K` is the key an operator still has to discover.
pub(crate) const COMPACT_IDLE_HINT: &str = "Enter send · Ctrl+K commands";

/// The width below which the composer is drawn as the compact layout: `02`
/// contrasts 100x30 with 80x24, and 80 is the compact one.
pub(crate) const COMPACT_WIDTH: usize = 100;

/// The hint row a reverse search draws in place of the state's own: the keys
/// that act on a search, and nothing else.
pub(crate) const SEARCH_HINT: &str = "Enter accept · Ctrl+R older · Esc cancel";

/// The prompt a reverse search draws on the composer's line (`Ctrl+R`): what is
/// being looked for, and the entry it found — or `(no match)` when its query
/// reaches nothing, so an operator sees the search is empty rather than stale.
pub(crate) fn search_prompt(model: &Model) -> Option<String> {
    let search = model.search()?;
    let found = model.search_found();
    let found = found.as_deref().unwrap_or("(no match)");
    Some(format!("(reverse-i-search)`{}`: {found}", search.query()))
}

/// The state `02` §7's table describes for this model.
///
/// A record and a pause are properties of the run, so they outrank the
/// composer's own contents; a held line outranks a running turn because it is
/// what the composer is showing; a connected database only matters when the
/// composer is otherwise idle.
pub(crate) fn state(model: &Model) -> State {
    if model.is_read_only_record() {
        State::ReadOnlyRecord
    } else if model.is_paused() {
        State::Paused
    } else if model.queued().is_some() {
        State::OneQueued
    } else if model.is_busy() {
        State::TurnRunning
    } else if model.database_name().is_some() {
        State::IdleWithDatabase
    } else {
        State::IdleWithoutDatabase
    }
}

/// The title `02` §7 gives this state.
pub(crate) fn title(state: State) -> &'static str {
    match state {
        State::IdleWithDatabase
        | State::IdleWithoutDatabase
        | State::Paused
        | State::ReadOnlyRecord => "Message",
        State::TurnRunning => "Message (queued on Enter)",
        State::OneQueued => "Queued",
        State::Disabled => "Message",
    }
}

/// The placeholder `02` §7 gives this state, or the held line when the state
/// is [`State::OneQueued`] — the table's placeholder there is the queued text
/// itself, shown dim.
pub(crate) fn placeholder(model: &Model) -> String {
    match state(model) {
        State::OneQueued => model.queued().unwrap_or_default().to_owned(),
        State::IdleWithDatabase => match model.database_name() {
            Some(name) => format!("Ask about {name} or describe a change"),
            // `state` only says `IdleWithDatabase` when a name is there; the
            // fallback keeps the copy readable rather than naming nothing.
            None => "Describe a change or ask about this code".to_owned(),
        },
        State::IdleWithoutDatabase => "Describe a change or ask about this code".to_owned(),
        State::TurnRunning => "Type a follow-up — it is sent when this turn ends".to_owned(),
        State::Paused => "Enter to continue, or type to redirect".to_owned(),
        State::ReadOnlyRecord => {
            "This is a 0.5.0 session — your next message starts a new run".to_owned()
        }
        State::Disabled => "Ask about the approval above".to_owned(),
    }
}

/// The hint row `02` §7 gives this state at `width` columns.
pub(crate) fn hint(state: State, width: usize) -> &'static str {
    match state {
        State::IdleWithDatabase | State::IdleWithoutDatabase => {
            if width < COMPACT_WIDTH {
                COMPACT_IDLE_HINT
            } else {
                IDLE_HINT
            }
        }
        State::TurnRunning => "Enter queue · Esc stop turn",
        State::OneQueued => QUEUED_HINT,
        State::Paused => "Enter continue · /status",
        State::ReadOnlyRecord => "Enter send",
        State::Disabled => "Enter send · Ctrl+K commands",
    }
}

/// The `/` popup's candidates for what is on the composer: today's completion,
/// called rather than copied.
pub(crate) fn completions(
    model: &Model,
    profiles: &[String],
) -> Option<(usize, usize, Vec<Candidate>)> {
    slash_candidates(model.composer(), profiles)
}

/// One run of a rendered row: its text and the token it is painted with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Run {
    pub(crate) text: String,
    pub(crate) token: Token,
}

/// One rendered row of the composer box.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Row {
    pub(crate) runs: Vec<Run>,
}

impl Row {
    /// The row's whole text, all runs together: what a snapshot or a test
    /// reads.
    pub(crate) fn text(&self) -> String {
        self.runs.iter().map(|run| run.text.as_str()).collect()
    }
}
