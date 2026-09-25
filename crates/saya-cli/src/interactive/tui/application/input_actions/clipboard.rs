use super::super::super::transcript::BlockKind;
use super::super::super::types::App;

impl App {
    /// Toggles selection mode. In selection mode the app releases the mouse so
    /// the terminal can drag-select and copy; the run loop reconciles the actual
    /// capture state. Wheel scrolling is unavailable while selecting.
    pub(crate) fn toggle_selection_mode(&mut self) {
        self.overlays.selection_mode = !self.overlays.selection_mode;
        let message = if self.overlays.selection_mode {
            // The copy keys filter thinking out; the terminal's own drag-select
            // cannot be filtered, so say so while the reasoning is on screen
            // rather than let the narrower guarantee read as a general one.
            if self
                .transcript
                .blocks()
                .iter()
                .any(|block| block.kind == BlockKind::Thinking)
            {
                "Selection mode on — drag to select and copy with your terminal. Thinking is on screen and your terminal can copy it. Ctrl+O to resume scrolling."
            } else {
                "Selection mode on — drag to select and copy with your terminal. Ctrl+O to resume scrolling."
            }
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
}
