//! Phase 8 packet 3: the derived index of results and the step affordance
//! over it.
//!
//! A result is a [`BlockKind::Table`] block; its honest address is
//! `(chapter, scope line)`, both already frozen into the block at completion.
//! Nothing new is stored: the index is derived from `blocks()` order and the
//! rendered row offsets, so no id, ordinal, or jump target is added to `Block`,
//! `Transcript`, or any persisted type. Stepping reads what is rendered and
//! never re-runs a query.
//!
//! The index lives here, beside `lines()`/`render()`, because an address is a
//! view coordinate — the row a result paints at — and only this layer knows
//! the fold-aware mapping from a block to that row.

use super::super::{BlockKind, Transcript};
use crate::interactive::tui::types::App;

/// One result's derived address: its block's place in `blocks()` order and the
/// chapter it belongs to. `Copy`, transient, and never persisted.
#[derive(Debug, Clone, Copy)]
struct ResultRef {
    block: usize,
    chapter: u32,
}

impl Transcript {
    /// Every result (table block) in transcript order. Derived from block
    /// kind alone — no field marks a result — so a resumed or replayed
    /// transcript enumerates identically.
    fn results(&self) -> Vec<ResultRef> {
        self.blocks()
            .iter()
            .enumerate()
            .filter(|(_, block)| block.kind == BlockKind::Table)
            .map(|(index, block)| ResultRef {
                block: index,
                chapter: block.chapter,
            })
            .collect()
    }

    /// Steps the viewport to the previous/next result: unfolds the target's
    /// chapter if folded, then places the result's first row at the viewport
    /// top. Returns false when there is no result in that direction, leaving
    /// the viewport untouched — the caller decides whether to speak.
    ///
    /// "Current" is the last result at or above the viewport top, or the last
    /// result while following the live edge (where no result starts above the
    /// top). Landing puts the target's top exactly at the viewport top, so a
    /// second step moves to its neighbour rather than hovering.
    pub(crate) fn step_result(&mut self, forward: bool, viewport: (u16, u16)) -> bool {
        let (width, height) = (viewport.0 as usize, viewport.1 as usize);
        let results = self.results();
        if results.is_empty() {
            return false;
        }
        let (rows, starts) = self.render(width);
        let rem = rows.len().saturating_sub(height);
        let top = rem.saturating_sub(self.scroll_up.min(rem));
        let current = if self.is_following_tail() {
            Some(results.len() - 1)
        } else {
            results
                .iter()
                .rposition(|result| starts[result.block] <= top)
        };
        let target = match (current, forward) {
            (Some(i), true) => (i + 1 < results.len()).then_some(i + 1),
            (Some(i), false) => i.checked_sub(1),
            (None, true) => Some(0),
            (None, false) => None,
        };
        let Some(target) = target else {
            return false;
        };
        let ResultRef { block, chapter } = results[target];
        // A folded chapter paints one summary row, so the result's rows are
        // not on screen to land on. Open it first, then re-derive the address:
        // unfolding shifts every row below the fold. Insert-only is not
        // available here — this is the deliberate open, never a fold.
        if self.is_folded(chapter) {
            self.toggle_chapter(chapter);
        }
        let (rows, starts) = self.render(width);
        let rem = rows.len().saturating_sub(height);
        let top = starts[block].min(rem);
        self.scroll_up = rem - top;
        // Landing back at the live edge means the rows below are now visible;
        // a stale count would be a lie. Landing on an older result keeps the
        // count: those rows are still unseen.
        if self.scroll_up == 0 {
            self.unseen_rows = 0;
        }
        true
    }
}

impl App {
    /// Steps to the previous/next result using the last rendered viewport,
    /// and says so plainly when the transcript holds none. A no-op at either
    /// end is silent: the viewport simply stays.
    pub(crate) fn step_result(&mut self, forward: bool) {
        if self.transcript.step_result(forward, self.viewport.get()) {
            return;
        }
        if self
            .transcript
            .blocks()
            .iter()
            .all(|block| block.kind != BlockKind::Table)
        {
            self.transcript
                .push(BlockKind::System, "No results to step to.");
        }
    }
}
