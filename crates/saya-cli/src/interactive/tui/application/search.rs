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
        });
    }

    pub(crate) fn search_char(&mut self, c: char) {
        if let Some(search) = self.overlays.search.as_mut() {
            search.query.push(c);
            search.selected = 0;
        }
    }

    pub(crate) fn search_backspace(&mut self) {
        if let Some(search) = self.overlays.search.as_mut() {
            search.query.pop();
            search.selected = 0;
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
    /// input buffer, or jump to the next transcript match.
    pub(crate) fn commit_search(&mut self) {
        let Some(search) = self.overlays.search.take() else {
            return;
        };
        match search.kind {
            SearchKind::History => {
                let matches = self.history.search(&search.query);
                if let Some(entry) = matches.get(search.selected) {
                    let text = entry.clone();
                    self.input.set_text(text);
                } else if !search.query.is_empty() {
                    self.input.set_text(search.query);
                }
            }
            SearchKind::Transcript => {
                if !search.query.is_empty() {
                    let (width, height) = self.viewport.get();
                    if !self.transcript.jump_to_match(
                        &search.query,
                        width as usize,
                        height as usize,
                    ) {
                        self.transcript.push(
                            BlockKind::System,
                            format!("no match for '{}'", search.query),
                        );
                    }
                }
                // Keep the query so repeated Ctrl+F re-opens with it? Simpler:
                // closed; pressing again starts fresh.
            }
        }
    }
}
