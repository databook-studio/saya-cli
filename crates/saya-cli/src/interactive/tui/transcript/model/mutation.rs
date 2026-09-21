//! The one mutation boundary for the transcript's blocks.
//!
//! Every path that mutates `blocks` — `push`, `append_delta`,
//! `buffer_tool_request`, `buffer_tool_completion`, `reset_delta`,
//! `reformat_last`, tool-group folding — must perform the same four-step
//! bookkeeping: capture the scrolled baseline, mutate, enforce bounds,
//! invalidate the cache and re-anchor the scrolled viewport (feeding
//! `unseen_rows`). The boundary is a single construct taking the mutation as
//! a closure so a new path cannot half-use it; a `begin()`/`commit()` pair
//! was rejected because forgetting the `commit` is exactly this bug. Nesting
//! cannot happen by construction: closures use the raw primitives
//! (`push_block`, direct field access) and never call a public mutating
//! path, so no `mutate` runs inside another's closure (`append_delta`'s
//! fallback arm calls `push_block`, not `push`).

use super::super::{MAX_BLOCKS, MAX_TOTAL_TEXT_BYTES, Transcript};

#[allow(dead_code)]
impl Transcript {
    /// Runs `f` as the whole of one block mutation: the complete bookkeeping
    /// of [`super::push`] around whatever blocks `f` changes. Returns
    /// `f`'s result.
    pub(crate) fn mutate<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        let baseline = (self.scroll_up != 0)
            .then(|| self.scrolled_baseline())
            .flatten();
        let result = f(self);
        self.enforce_bounds();
        self.after_scrolled_append(baseline);
        result
    }

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
    /// surface as today — and nothing is counted: rows already on screen are
    /// not "new". `before`/`after` are painted lengths at the cached width;
    /// the append, never a later width-driven re-wrap, sits between the two
    /// measures. The length is measured around the *whole* mutation (append
    /// plus `enforce_bounds`), so top-dropped rows shrink `after` and shrink
    /// the bump with it — and `view()`'s `scroll_up.min(rem)` clamp absorbs
    /// any overshoot instead of running past the start. The same growth is
    /// the unseen-row count the "new lines below" indicator reads; this is
    /// its one raise site.
    fn freeze_scrolled_view(&mut self, before: usize, after: usize) {
        if self.scroll_up == 0 {
            return;
        }
        let growth = after.saturating_sub(before);
        self.scroll_up = self.scroll_up.saturating_add(growth);
        self.unseen_rows = self.unseen_rows.saturating_add(growth);
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

    /// Compensates `scroll_up` for a just-completed append around the cached
    /// baseline: re-wrap at the same width, bump by the growth, leave the
    /// cache warm. `None` means no stored view yet — plain invalidation, and
    /// no compensation: without a rental there is nothing to freeze. A
    /// shrinking mutation (`after < before`) computes a zero growth, so
    /// pure-removal paths fold and discard without moving the anchor.
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
}

#[cfg(test)]
#[path = "mutation_tests.rs"]
mod mutation_tests;
