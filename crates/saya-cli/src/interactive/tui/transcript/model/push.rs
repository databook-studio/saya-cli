use super::super::{MAX_BLOCKS, MAX_TOTAL_TEXT_BYTES, Transcript, chapters};
use super::{Block, BlockKind};

#[allow(dead_code)]
impl Transcript {
    pub(super) fn enforce_bounds(&mut self) {
        let mut bytes: usize = self.blocks.iter().map(|b| b.text.len()).sum();
        let mut drop = 0;
        while self.blocks.len().saturating_sub(drop) > MAX_BLOCKS
            || (bytes > MAX_TOTAL_TEXT_BYTES && drop < self.blocks.len())
        {
            bytes = bytes.saturating_sub(self.blocks[drop].text.len());
            drop += 1;
        }
        if drop > 0 {
            self.blocks.drain(..drop);
        }
    }

    /// Freezes the scrolled reader's window across an append: `view()` windows
    /// tail-relative (`start = painted - height - scroll_up`), so appended
    /// rows slide the window down by the growth while `scroll_up > 0`.
    /// Bumping `scroll_up` by the growth keeps the top anchor on the same
    /// row. At the tail (`scroll_up == 0`) nothing moves — the new rows
    /// surface as today. `before`/`after` are painted lengths at the cached
    /// width; the append (`push`/`append_delta` mutate blocks), never a
    /// later width-driven re-wrap, sits between the two measures. The length
    /// is measured around the *whole* mutation (append plus `enforce_bounds`),
    /// so top-dropped rows shrink `after` and shrink the bump with it — and
    /// `view()`'s `scroll_up.min(rem)` clamp absorbs any overshoot instead of
    /// running past the start.
    fn freeze_scrolled_view(&mut self, before: usize, after: usize) {
        if self.scroll_up == 0 {
            return;
        }
        self.scroll_up = self.scroll_up.saturating_add(after.saturating_sub(before));
    }

    /// Painted-length baseline for [`freeze_scrolled_view`]: the cached
    /// `(width, len)` when a stored view exists, `None` on a mutation that
    /// has not rendered yet. `None` is the honest answer — not "measure at
    /// 80", which would chase a width nobody is looking at.
    fn scrolled_baseline(&self) -> Option<(usize, usize)> {
        self.cache
            .borrow()
            .as_ref()
            .map(|(width, rows)| (*width, rows.len()))
    }

    pub(crate) fn push(&mut self, kind: BlockKind, text: impl Into<String>) {
        let baseline = (self.scroll_up != 0)
            .then(|| self.scrolled_baseline())
            .flatten();
        self.push_block(kind, text.into());
        self.after_scrolled_append(baseline);
    }

    pub(crate) fn append_delta(&mut self, kind: BlockKind, delta: &str) {
        if !self.blocks.last().is_some_and(|last| last.kind == kind) {
            self.push(kind, delta);
            return;
        }
        let baseline = (self.scroll_up != 0)
            .then(|| self.scrolled_baseline())
            .flatten();
        let Some(last) = self.blocks.last_mut().filter(|last| last.kind == kind) else {
            return;
        };
        last.text.push_str(delta);
        self.enforce_bounds();
        self.after_scrolled_append(baseline);
    }

    fn push_block(&mut self, kind: BlockKind, text: String) {
        let chapter = chapters::chapter_for(&self.blocks, kind);
        self.blocks.push(Block {
            kind,
            text,
            chapter,
            group: None,
        });
        self.enforce_bounds();
    }

    /// Compensates `scroll_up` for a just-completed append around the cached
    /// baseline: re-wrap at the same width, bump by the growth, leave the
    /// cache warm. `None` means no stored view yet — plain invalidation, and
    /// no compensation: without a rental there is nothing to freeze.
    fn after_scrolled_append(&mut self, baseline: Option<(usize, usize)>) {
        match baseline {
            Some((width, before)) => {
                self.invalidate_cache();
                let after = self.lines(width).len();
                self.freeze_scrolled_view(before, after);
            }
            None => self.invalidate_cache(),
        }
    }

    /// Clears the text accumulated in the trailing block of `kind` — exactly
    /// the block the next [`append_delta`] of that kind would extend — so a
    /// re-streamed answer **replaces** what streamed so far instead of
    /// appending (the `TurnReset` retry path). No-op when the trailing block
    /// is not of `kind` (nothing streamed yet to discard).
    pub(crate) fn reset_delta(&mut self, kind: BlockKind) {
        if let Some(last) = self.blocks.last_mut().filter(|last| last.kind == kind) {
            last.text.clear();
            self.invalidate_cache();
        }
    }

    pub(crate) fn reformat_last(&mut self, kind: BlockKind, f: impl FnOnce(&str) -> String) {
        if let Some(block) = self.blocks.iter_mut().rev().find(|b| b.kind == kind) {
            block.text = f(&block.text);
            self.invalidate_cache();
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.blocks.clear();
        self.folded.clear();
        self.scroll_up = 0;
        self.invalidate_cache();
    }

    pub(crate) fn blocks(&self) -> &[Block] {
        &self.blocks
    }
}

#[cfg(test)]
#[path = "push_tests.rs"]
mod push_tests;
