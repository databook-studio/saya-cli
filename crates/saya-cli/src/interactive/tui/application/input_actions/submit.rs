use super::super::super::transcript::BlockKind;
use super::super::super::types::App;

impl App {
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
        // Auto-fold the chapter that just finished, before the new `User`
        // block opens the next one — the only automatic fold in the app
        // (never on a timer, scroll, completion, or resume). Guarded: only
        // while following the tail, never with a pending approval (an
        // unresolved decision whose context folding could hide), and only the
        // previous chapter via insert-only semantics (a chapter the user
        // reopened stays open). The busy path above returns early, so a queued
        // prompt — no new chapter — folds nothing.
        if self.request.pending_approval.is_none() {
            self.transcript.auto_fold_finished_chapter();
        }
        self.transcript.push(BlockKind::User, line.clone());
        self.transcript.scroll_to_bottom();
        self.pending = Some(line);
    }
}
