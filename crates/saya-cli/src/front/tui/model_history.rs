//! The composer's input history, and the reverse search over it (`02` §7).
//!
//! The ring, its `input_history` file and its `--param` redaction are today's
//! — `interactive::tui::history`, re-exported rather than copied — so a line
//! recalled here is the line the old surface would recall, and a `--param`
//! value never reaches the file or the screen from either surface. What this
//! module adds is the composer's side of it: the readline-style walk `Up` and
//! `Down` make, the draft `Down` restores past the newest entry, and the
//! reverse search `Ctrl+R` opens.
//!
//! All of it is view state. A search is never persisted, and the draft it was
//! opened over is the composer's own — an accepted search replaces it, a
//! cancelled one leaves it exactly as it was.

use super::model::Model;

/// A reverse search in progress over the ring: the query typed so far, and
/// which of its matches (newest first) is being shown.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Search {
    query: String,
    picked: usize,
}

impl Search {
    /// The query typed so far.
    pub(crate) fn query(&self) -> &str {
        &self.query
    }

    /// Which match, newest first, the search shows.
    pub(crate) const fn picked(&self) -> usize {
        self.picked
    }

    /// Narrows the search by one character, from its newest match again: a
    /// longer query is a different search, not the same one further down.
    fn narrow(&mut self, character: char) {
        self.query.push(character);
        self.picked = 0;
    }
}

impl Model {
    /// The ring this composer recalls and searches.
    pub(crate) fn history(&self) -> &crate::interactive::tui::History {
        &self.history
    }

    /// Takes the ring the launch loaded: the `input_history` file, bounded and
    /// de-duplicated as today's surface reads it.
    pub(crate) fn open_history(&mut self) {
        self.history = crate::interactive::tui::History::load();
    }

    /// Records one submitted line: the ring redacts its `--param` values on the
    /// way in (`History::push`), so a line recalled or persisted shows
    /// `--param name=…` while the run received the value.
    pub(crate) fn remember(&mut self, line: &str) {
        self.history.push(line);
    }

    /// Recalls the previous entry into the composer (Up). Recall only fills the
    /// line — it never submits — and the first Up from a fresh draft stashes
    /// that draft, readline-style, so Down can bring it back.
    pub(crate) fn recall_previous(&mut self) {
        let entering = !self.history.navigating();
        let Some(entry) = self.history.previous().map(str::to_owned) else {
            return;
        };
        if entering {
            let draft = self.composer.clone();
            self.history.stash_draft(&draft);
        }
        self.composer = entry;
    }

    /// Recalls the next entry (Down). Stepping back past the newest entry
    /// restores the stashed draft; with no history position at all, Down leaves
    /// the draft alone — it never wipes what the operator typed.
    pub(crate) fn recall_next(&mut self) {
        match self.history.next().map(str::to_owned) {
            Some(entry) => self.composer = entry,
            None => {
                if let Some(draft) = self.history.take_stash() {
                    self.composer = draft;
                }
            }
        }
    }

    /// Opens the reverse search (`Ctrl+R`) on an empty query: its newest match
    /// is what the composer shows until the operator types.
    pub(crate) fn open_search(&mut self) {
        self.search = Some(Search::default());
    }

    /// The search that is open, if one is.
    pub(crate) fn search(&self) -> Option<&Search> {
        self.search.as_ref()
    }

    /// The entries the open search's query matches, newest first.
    pub(crate) fn search_matches(&self) -> Vec<String> {
        match self.search.as_ref() {
            Some(search) => self.history.search(search.query()),
            None => Vec::new(),
        }
    }

    /// The entry the open search is showing, if its query matches anything.
    pub(crate) fn search_found(&self) -> Option<String> {
        let picked = self.search.as_ref()?.picked();
        self.search_matches().into_iter().nth(picked)
    }

    /// Adds one character to the open search's query.
    pub(crate) fn search_type(&mut self, character: char) {
        if let Some(search) = &mut self.search {
            search.narrow(character);
        }
    }

    /// Removes the open search's last query character: the search widens again,
    /// from its newest match.
    pub(crate) fn search_backspace(&mut self) {
        if let Some(search) = &mut self.search {
            search.query.pop();
            search.picked = 0;
        }
    }

    /// Steps the open search one match older, as a second `Ctrl+R` does. It
    /// stops at the oldest match rather than wrapping: the operator asked for
    /// something older, and there is nothing older to show.
    pub(crate) fn search_older(&mut self) {
        let matches = self.search_matches().len();
        if let Some(search) = &mut self.search {
            let last = matches.saturating_sub(1);
            search.picked = (search.picked + 1).min(last);
        }
    }

    /// Takes the entry the search shows onto the composer's line and closes it
    /// (`Enter`). The walk through the ring ends here: what is on the line is
    /// new, so a later Down must not resurrect the draft it replaced.
    pub(crate) fn accept_search(&mut self) {
        if let Some(found) = self.search_found() {
            self.composer = found;
            self.history.reset();
        }
        self.search = None;
    }

    /// Closes the search, leaving the composer's line exactly as it was
    /// (`Esc`).
    pub(crate) fn close_search(&mut self) {
        self.search = None;
    }
}
