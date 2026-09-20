use super::super::super::transcript::BlockKind;
use super::super::super::types::App;

impl App {
    /// One-line preview of a queued prompt for the queue notice. Newlines
    /// collapse to spaces so the notice stays one visual line; long prompts
    /// truncate with an ellipsis. `pending` keeps the full text — the
    /// preview never changes what runs.
    fn queued_preview(line: &str) -> String {
        const PREVIEW_CHARS: usize = 120;
        let flat: String = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if flat.chars().count() <= PREVIEW_CHARS {
            return flat;
        }
        let kept: String = flat.chars().take(PREVIEW_CHARS).collect();
        format!("{kept}…")
    }

    /// Drops the queued prompt without submitting anything. The active
    /// request is untouched, the earlier transcript stands, and the input
    /// draft survives. No-op when nothing is queued.
    pub(crate) fn drop_queued_prompt(&mut self) {
        if self.pending.take().is_some() {
            self.transcript
                .push(BlockKind::System, "Dropped the queued prompt.");
            self.transcript.scroll_to_bottom();
        }
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
            // request finishes. One slot — resubmitting replaces it. The
            // notice quotes the prompt (truncated) so the queue is visible;
            // it stays a `System` block — a `User` block would open a chapter
            // and read as the active task. When the stop was already asked
            // (the stream token is cancelled but the worker has not settled),
            // the notice says the run waits for the stop confirmation.
            let replaced = self.pending.is_some();
            let preview = Self::queued_preview(&line);
            let stop_pending = self
                .request
                .stream
                .as_ref()
                .is_some_and(|stream| stream.cancel.is_cancelled());
            self.pending = Some(line);
            let tail = if stop_pending {
                format!("Queued — runs once the stop is confirmed:\n  {preview}")
            } else if replaced {
                format!(
                    "Queued (replaced the earlier queued prompt) — runs after the current request:\n  {preview}"
                )
            } else {
                format!("Queued — runs when the current request finishes:\n  {preview}")
            };
            self.transcript.push(BlockKind::System, tail);
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
