//! The one mutation boundary for the transcript's blocks.
//!
//! Every path that mutates `blocks` — `push`, `append_delta`,
//! `buffer_tool_request`, `buffer_tool_completion`, `reset_delta`,
//! `reformat_last`, tool-group folding — must perform the same five-step
//! bookkeeping: capture the scrolled baseline, mutate, measure the closure's
//! own painted length, enforce bounds, re-measure and re-anchor the scrolled
//! viewport (feeding `unseen_rows`). The boundary is a single construct taking
//! the mutation as a closure so a new path cannot half-use it; a `begin()`/
//! `commit()` pair was rejected because forgetting the `commit` is exactly
//! this bug. Nesting cannot happen by construction: closures use the raw
//! primitives (`push_block`, direct field access) and never call a public
//! mutating path, so no `mutate` runs inside another's closure
//! (`append_delta`'s fallback arm calls `push_block`, not `push`).

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
        // The closure's own painted length, measured BEFORE `enforce_bounds`:
        // this boundary serves two different shrinks and they need opposite
        // handling. A tail shrink (a fold or discard below the reader) must
        // lower `scroll_up` by the delta, or `start` falls with `rem` and the
        // window walks backwards through history (re-audit R03). Prefix
        // eviction (bounds draining rows off the front) must not move
        // `scroll_up` at all: the whole array shifts down by the dropped
        // count, so `start` falling by that same count already lands on the
        // same logical row — adjusting here would double-count. A single
        // signed delta measured over the whole mutation cannot tell the two
        // apart; only the pre-eviction measure can.
        let measured = baseline.map(|(width, before)| {
            self.invalidate_cache();
            let mid = self.lines(width).len();
            (width, before, mid)
        });
        let evicted = self.enforce_bounds();
        self.after_scrolled_mutation(measured, evicted);
        result
    }

    pub(super) fn enforce_bounds(&mut self) -> bool {
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
            return true;
        }
        false
    }

    /// Freezes the scrolled reader's window across one mutation. `view()`
    /// windows tail-relative (`start = painted - height - scroll_up`), so
    /// appended rows slide the window down by the growth while
    /// `scroll_up > 0`: bumping `scroll_up` by the growth keeps the top anchor
    /// on the same row. A tail shrink — a fold or discard removing rows below
    /// the reader — is the opposite: `scroll_up` must FALL by the shrink, or
    /// `start` falls with `rem` and the window walks backwards through
    /// history under a reader who pressed nothing. Only the closure's own
    /// delta moves the anchor; `enforce_bounds`' prefix eviction is absorbed
    /// by the index shift, and `view()`'s `scroll_up.min(rem)` clamp stays
    /// the backstop against anchoring past the start. At the tail
    /// (`scroll_up == 0`) nothing moves — new rows surface as today — and
    /// nothing is counted: rows already on screen are not "new".
    ///
    /// `unseen_rows` rises by a growth and falls by a shrink, clamped to
    /// `scroll_up`: a collapsing group is not new activity, and the rows the
    /// count was raised for are usually the very rows the fold removes — the
    /// promise must shrink with them or the count stops tracking anything
    /// painted. The clamp is the hard bound: the tail-relative window sits
    /// exactly `scroll_up` rows above the tail, so `unseen_rows ≤ scroll_up`
    /// is "never exceed the rows actually below the window".
    fn freeze_scrolled_view(&mut self, before: usize, mid: usize) {
        if self.scroll_up == 0 {
            return;
        }
        if mid >= before {
            let growth = mid - before;
            self.scroll_up = self.scroll_up.saturating_add(growth);
            self.unseen_rows = self.unseen_rows.saturating_add(growth);
        } else {
            let shrink = before - mid;
            self.scroll_up = self.scroll_up.saturating_sub(shrink);
            self.unseen_rows = self.unseen_rows.saturating_sub(shrink).min(self.scroll_up);
        }
        if self.scroll_up == 0 {
            // A shrink can pull the window back onto the tail: everything
            // below is then visible, the same rule as `scroll_down`.
            self.unseen_rows = 0;
        }
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

    /// Compensates `scroll_up` for a just-completed mutation around the
    /// cached baseline: re-wrap at the same width, apply the closure's own
    /// painted delta (`before` → `mid`, the only delta that moves the
    /// anchor), leave the cache warm. `None` means no stored view yet — plain
    /// invalidation, and no compensation: without a rental there is nothing
    /// to freeze. `evicted` says `enforce_bounds` drained rows off the front;
    /// the cache is then stale and the re-measure is a real re-wrap — the one
    /// extra pass a scrolled mutation pays while evicting (without eviction
    /// the post-bounds measure is a cache hit). No cache or index is added to
    /// avoid it. A shrinking mutation is no longer a zero growth: pure-removal
    /// paths fold and discard with the anchor lowered by the exact shrink, so
    /// the window stays on the rows the reader was reading.
    fn after_scrolled_mutation(&mut self, measured: Option<(usize, usize, usize)>, evicted: bool) {
        match measured {
            Some((width, before, mid)) => {
                if evicted {
                    self.invalidate_cache();
                }
                // Not just the assertion's input: this re-wrap repopulates the
                // cache the measure above invalidated, which is the warm-cache
                // behaviour the boundary has always left behind. It runs in
                // release too — do not fold it into the `debug_assert!`.
                let after = self.lines(width).len();
                debug_assert!(after <= mid, "bounds enforcement only removes rows");
                self.freeze_scrolled_view(before, mid);
            }
            None => self.invalidate_cache(),
        }
    }
}

#[cfg(test)]
#[path = "mutation_tests.rs"]
mod mutation_tests;

#[cfg(test)]
#[path = "shrink_anchor_tests.rs"]
mod shrink_anchor_tests;
