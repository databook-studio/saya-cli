//! Input editing, autocomplete popup, history recall, and clipboard queueing.

use super::super::atref;
use super::super::complete;
use super::super::transcript::BlockKind;
use super::super::types::{App, Menu};

impl App {
    /// Toggles selection mode. In selection mode the app releases the mouse so
    /// the terminal can drag-select and copy; the run loop reconciles the actual
    /// capture state. Wheel scrolling is unavailable while selecting.
    pub(crate) fn toggle_selection_mode(&mut self) {
        self.overlays.selection_mode = !self.overlays.selection_mode;
        let message = if self.overlays.selection_mode {
            "Selection mode on — drag to select and copy with your terminal. Ctrl+O to resume scrolling."
        } else {
            "Selection mode off — mouse wheel scrolls again."
        };
        self.transcript.push(BlockKind::System, message);
    }

    /// Queues the most recent assistant answer for the clipboard (F3).
    pub(crate) fn copy_last_answer(&mut self) {
        if self.clipboard_copy.is_some() || self.pending_clipboard.is_some() {
            self.transcript
                .push(BlockKind::System, "Clipboard copy already in progress.");
            return;
        }
        match self
            .transcript
            .blocks()
            .iter()
            .rev()
            .find(|block| block.kind == BlockKind::Assistant)
        {
            Some(block) => {
                self.pending_clipboard = Some(block.text.clone());
            }
            None => self
                .transcript
                .push(BlockKind::System, "No answer to copy yet."),
        }
    }

    /// Queues the whole transcript for the clipboard (F4).
    ///
    /// The model's chain-of-thought is excluded: it restates row values and
    /// column contents in prose, and the clipboard is a channel off-screen —
    /// putting model reasoning that may restate database contents on the system
    /// clipboard is a sharper exposure than showing it on screen to the person
    /// already reading the answer. `/help thinking` names this so it is not a
    /// surprise.
    pub(crate) fn copy_transcript(&mut self) {
        if self.clipboard_copy.is_some() || self.pending_clipboard.is_some() {
            self.transcript
                .push(BlockKind::System, "Clipboard copy already in progress.");
            return;
        }
        let text = self
            .transcript
            .blocks()
            .iter()
            .filter(|block| block.kind != BlockKind::Thinking)
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        if text.is_empty() {
            self.transcript
                .push(BlockKind::System, "Nothing to copy yet.");
            return;
        }
        self.pending_clipboard = Some(text);
    }

    /// Recalls the previous history entry into the input (Up).
    pub(crate) fn history_prev(&mut self) {
        if let Some(entry) = self.history.previous() {
            let entry = entry.to_string();
            self.input.set_text(entry);
            self.refresh_menu();
        }
    }

    /// Recalls the next history entry, or restores an empty line (Down).
    pub(crate) fn history_next(&mut self) {
        match self.history.next() {
            Some(entry) => self.input.set_text(entry.to_string()),
            None => self.input.clear(),
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

    /// Pushes a blank separator line, unless the transcript is empty or already ends in one.
    fn push_spacer(&mut self) {
        match self.transcript.blocks().last() {
            None => {}
            Some(last) if last.text.is_empty() => {}
            Some(_) => self.transcript.push(BlockKind::System, String::new()),
        }
    }

    /// Captures the current line for dispatch and clears the input.
    pub(crate) fn submit(&mut self) {
        let line = self.input.text().trim_end().to_string();
        self.input.clear();
        self.overlays.menu = None;
        if line.is_empty() {
            return;
        }
        self.history.push(&line);
        if self.is_busy() {
            // Queue instead of dropping: the prompt runs when the current
            // request finishes. One slot — resubmitting replaces it.
            let replaced = self.pending.is_some();
            self.pending = Some(line);
            self.transcript.push(
                BlockKind::System,
                if replaced {
                    "Queued (replaced the earlier queued prompt) — runs after the current request."
                } else {
                    "Queued — runs as soon as the current request finishes."
                },
            );
            self.transcript.scroll_to_bottom();
            return;
        }
        self.push_spacer();
        self.transcript.push(BlockKind::User, line.clone());
        self.transcript.scroll_to_bottom();
        self.pending = Some(line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interactive::tui::application::tests_support::idle_app;

    /// Copying the transcript excludes the model's chain-of-thought. Reasoning
    /// restates row values and column contents in prose, and the clipboard is a
    /// channel off-screen — putting it on the system clipboard is a sharper
    /// exposure than showing it to the person already reading the answer. The
    /// user and assistant blocks are copied; the thinking block is not.
    #[test]
    fn copy_transcript_excludes_thinking_blocks() {
        let mut app = idle_app();
        app.transcript.push(BlockKind::User, "what is the answer");
        app.transcript.push(
            BlockKind::Thinking,
            "the secret chain-of-thought about row values",
        );
        app.transcript
            .push(BlockKind::Assistant, "the answer is 42");

        app.copy_transcript();
        let copied = app.pending_clipboard.expect("transcript was queued");
        assert!(
            copied.contains("the answer is 42"),
            "assistant text must be copied: {copied}"
        );
        assert!(
            copied.contains("what is the answer"),
            "user text must be copied: {copied}"
        );
        assert!(
            !copied.contains("the secret chain-of-thought about row values"),
            "thinking must not reach the clipboard: {copied}"
        );
    }

    /// `copy_last_answer` finds the assistant block, not a thinking block, so the
    /// chain-of-thought never reaches the clipboard even when it is the most
    /// recent block.
    #[test]
    fn copy_last_answer_skips_thinking_blocks() {
        let mut app = idle_app();
        app.transcript
            .push(BlockKind::Assistant, "the answer is 42");
        app.transcript
            .push(BlockKind::Thinking, "the secret chain-of-thought");

        app.copy_last_answer();
        let copied = app.pending_clipboard.expect("answer was queued");
        assert_eq!(copied, "the answer is 42");
    }
}
