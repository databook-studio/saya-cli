//! Ctrl+R (input history) and Ctrl+F (transcript find) overlays.

use super::super::transcript::BlockKind;
use super::super::types::{App, SearchKind};
use crate::interactive::tui::types::SearchOverlay;

impl App {
    pub(crate) fn open_search(&mut self, kind: SearchKind) {
        self.overlays.search = Some(SearchOverlay {
            kind,
            query: String::new(),
            selected: 0,
            last_match: None,
        });
    }

    pub(crate) fn search_char(&mut self, c: char) {
        if let Some(search) = self.overlays.search.as_mut() {
            search.query.push(c);
            search.selected = 0;
            // An edit changes the result set: restart from the viewport top.
            search.last_match = None;
        }
    }

    pub(crate) fn search_backspace(&mut self) {
        if let Some(search) = self.overlays.search.as_mut() {
            search.query.pop();
            search.selected = 0;
            search.last_match = None;
        }
    }

    pub(crate) fn search_move(&mut self, delta: i32) {
        let len = match self.overlays.search.as_ref() {
            Some(search) => self.search_matches(search).len(),
            None => return,
        };
        if let Some(search) = self.overlays.search.as_mut() {
            if len == 0 {
                return;
            }
            let current = search.selected as i32 + delta;
            search.selected = (current.clamp(0, len as i32 - 1)) as usize;
        }
    }

    /// Filtered candidates for the overlay (history mode; empty in find mode).
    pub(crate) fn search_matches(&self, search: &SearchOverlay) -> Vec<String> {
        match search.kind {
            SearchKind::History => self.history.search(&search.query),
            SearchKind::Transcript => Vec::new(),
        }
    }

    pub(crate) fn close_search(&mut self) {
        self.overlays.search = None;
    }

    /// Enter in the overlay: pick the highlighted history entry into the
    /// input buffer, or jump to the next transcript match. The transcript
    /// overlay stays open so repeated Enter walks the matches — the first
    /// Enter lands on the first match at/after the viewport top, and each
    /// later Enter continues from one line past the last landing.
    pub(crate) fn commit_search(&mut self) {
        // Read what we need from a short-lived borrow so the mutable calls
        // below (take / find_next_match / push) do not overlap it.
        let (kind, query, selected, last_match) = match self.overlays.search.as_ref() {
            Some(search) => (
                search.kind,
                search.query.clone(),
                search.selected,
                search.last_match,
            ),
            None => return,
        };
        match kind {
            SearchKind::History => {
                // History mode closes the overlay: take it so the picked entry
                // is committed to the input and the user keeps typing.
                let matches = self.history.search(&query);
                if let Some(entry) = matches.get(selected) {
                    self.input.set_text(entry.clone());
                } else if !query.is_empty() {
                    self.input.set_text(query);
                }
                self.overlays.search = None;
            }
            SearchKind::Transcript => {
                if query.is_empty() {
                    return;
                }
                let (width, height) = self.viewport.get();
                match self.find_next_match(&query, last_match, width as usize, height as usize) {
                    Some(idx) => {
                        if let Some(search) = self.overlays.search.as_mut() {
                            search.last_match = Some(idx);
                        }
                    }
                    None => {
                        // No (further) match. Reset so the next Enter restarts
                        // from the viewport top instead of a dead scan window.
                        if let Some(search) = self.overlays.search.as_mut() {
                            search.last_match = None;
                        }
                        self.transcript
                            .push(BlockKind::System, format!("no match for '{}'", query));
                    }
                }
            }
        }
    }

    /// Finds the next transcript line matching `needle` and scrolls it into
    /// view. When `after` is `Some(idx)` the scan begins one line past that
    /// index (find-next); otherwise it begins at the current viewport top
    /// (first search). The matched line is pinned to the last visible row.
    /// The first search reuses [`Transcript::jump_to_match`] for placement so
    /// that helper stays the single owner of the viewport-pin logic; find-next
    /// (which `jump_to_match` cannot target at a specific start line) positions
    /// by scrolling up from the tail. Returns the landed wrapped-line index, or
    /// `None` when there is no match.
    fn find_next_match(
        &mut self,
        needle: &str,
        after: Option<usize>,
        width: usize,
        height: usize,
    ) -> Option<usize> {
        let lines = self.transcript.wrapped(width);
        let total = lines.len();
        if total == 0 || height == 0 {
            return None;
        }
        let needle = needle.to_lowercase();
        let (_, current_top) = self.transcript.scroll_metrics(width, height);
        let start = after.map(|idx| (idx + 1) % total).unwrap_or(current_top);
        let idx = (0..total)
            .map(|offset| (start + offset) % total)
            .find(|&i| !lines[i].is_label && lines[i].text.to_lowercase().contains(&needle))?;
        if after.is_none() {
            // First search: `jump_to_match` scans from the same current top and
            // pins the match, so delegate the placement to it.
            self.transcript.jump_to_match(&needle, width, height);
        } else {
            // Find-next: pin the match to the last visible row by scrolling up
            // from the tail, the same placement `jump_to_match` uses.
            let max = total.saturating_sub(height);
            self.transcript.scroll_to_bottom();
            self.transcript
                .scroll_up((total - 1 - idx).min(max), width, height);
        }
        Some(idx)
    }
}

#[cfg(test)]
#[path = "search_tests.rs"]
mod tests;
