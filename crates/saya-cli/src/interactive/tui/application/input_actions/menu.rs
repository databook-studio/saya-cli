use super::super::super::atref;
use super::super::super::complete;
use super::super::super::types::{App, Menu};

impl App {
    /// Recalls the previous history entry into the input (Up). Recall only
    /// fills the draft — it never submits. The first Up from a fresh line
    /// (unset history cursor) stashes the in-progress draft, readline-style,
    /// so stepping back down past the newest entry can restore it.
    pub(crate) fn history_prev(&mut self) {
        let entering = !self.history.navigating();
        if let Some(entry) = self.history.previous() {
            let entry = entry.to_string();
            if entering {
                self.history.stash_draft(self.input.text());
            }
            self.input.set_text(entry);
            self.refresh_menu();
        }
    }

    /// Recalls the next history entry (Down). Stepping back past the newest
    /// entry restores the stashed draft; with no history position at all,
    /// Down leaves the draft alone — it never wipes what the user typed.
    pub(crate) fn history_next(&mut self) {
        match self.history.next() {
            Some(entry) => self.input.set_text(entry.to_string()),
            None => {
                if let Some(draft) = self.history.take_stash() {
                    self.input.set_text(draft);
                }
            }
        }
        self.refresh_menu();
    }

    /// Recomputes the popup: slash commands when the line starts with '/',
    /// otherwise `@table` references from the schema being typed at the cursor.
    pub(crate) fn refresh_menu(&mut self) {
        let cursor = self.input.cursor();
        let found = complete::slash_candidates(self.input.text(), &self.profiles)
            .or_else(|| atref::at_candidates(self.input.text(), cursor, &self.at_refs));
        self.overlays.menu = found.map(|(start, end, candidates)| Menu {
            start,
            end,
            candidates,
            selected: 0,
        });
    }

    /// Moves the popup selection by `delta`, clamped.
    pub(crate) fn menu_move(&mut self, delta: isize) {
        if let Some(menu) = &mut self.overlays.menu {
            let len = menu.candidates.len();
            if len == 0 {
                return;
            }
            let next = (menu.selected as isize + delta).clamp(0, len as isize - 1);
            menu.selected = next as usize;
        }
    }

    /// Replaces the completed token with the highlighted candidate. Completing a
    /// command word re-opens the popup for its argument; completing an argument
    /// value closes the popup so the next Enter submits.
    pub(crate) fn accept_selected(&mut self) {
        let mut completed_command = false;
        if let Some(menu) = &self.overlays.menu
            && let Some(candidate) = menu.candidates.get(menu.selected)
        {
            let chars: Vec<char> = self.input.text().chars().collect();
            let start = menu.start.min(chars.len());
            let end = menu.end.min(chars.len());
            let before: String = chars[..start].iter().collect();
            let after: String = chars[end..].iter().collect();
            completed_command = candidate.value.starts_with('/');
            let mut text = before;
            text.push_str(&candidate.value);
            if completed_command {
                text.push(' ');
            }
            text.push_str(&after);
            self.input.set_text(text);
        }
        if completed_command {
            self.refresh_menu();
        } else {
            self.overlays.menu = None;
        }
    }

    /// Inserts pasted text into the input without submitting (normalizing
    /// newlines), so multi-line pastes land in the box instead of running.
    pub(crate) fn paste(&mut self, text: &str) {
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        self.input.insert_str(&normalized);
        self.refresh_menu();
    }

    /// Toggles the most recent collapsed tool group between its one-line
    /// summary and its full per-call sequence. No-op when the transcript
    /// holds no group: the user pressed the key with nothing to expand, and
    /// an honest silence beats a notice line that would itself scroll the
    /// transcript they are reading.
    pub(crate) fn toggle_tool_group(&mut self) -> bool {
        self.transcript.toggle_latest_group()
    }

    /// Toggles the newest foldable chapter (see
    /// [`Transcript::toggle_latest_chapter`]). Enter on an empty line falls
    /// through to this only when no tool group toggled.
    pub(crate) fn toggle_latest_chapter(&mut self) -> bool {
        self.transcript.toggle_latest_chapter()
    }
}
