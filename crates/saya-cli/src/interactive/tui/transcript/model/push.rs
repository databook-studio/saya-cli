use super::super::{Transcript, chapters};
use super::{Block, BlockKind};

#[allow(dead_code)]
impl Transcript {
    /// Appends one block through [`Transcript::mutate`], the one mutation
    /// boundary: baseline, bounds, invalidation, viewport anchor.
    pub(crate) fn push(&mut self, kind: BlockKind, text: impl Into<String>) {
        self.mutate(|t| t.push_block(kind, text.into()));
    }

    /// Extends the trailing block of `kind`, or opens one — through
    /// [`Transcript::mutate`]. The fallback arm calls the raw `push_block`,
    /// never [`Transcript::push`], so the boundary never nests and the
    /// baseline is captured once.
    pub(crate) fn append_delta(&mut self, kind: BlockKind, delta: &str) {
        self.mutate(|t| {
            if let Some(last) = t.blocks.last_mut().filter(|last| last.kind == kind) {
                last.text.push_str(delta);
            } else {
                t.push_block(kind, delta.to_string());
            }
        });
    }

    fn push_block(&mut self, kind: BlockKind, text: String) {
        let chapter = chapters::chapter_for(&self.blocks, kind);
        self.blocks.push(Block {
            kind,
            text,
            chapter,
            group: None,
        });
    }

    /// Clears the text accumulated in the trailing block of `kind` — exactly
    /// the block the next [`append_delta`] of that kind would extend — so a
    /// re-streamed answer **replaces** what streamed so far instead of
    /// appending (the `TurnReset` retry path). No-op when the trailing block
    /// is not of `kind` (nothing streamed yet to discard). Clearing only
    /// shrinks, so the boundary's anchor is a no-op here.
    pub(crate) fn reset_delta(&mut self, kind: BlockKind) {
        self.mutate(|t| {
            if let Some(last) = t.blocks.last_mut().filter(|last| last.kind == kind) {
                last.text.clear();
            }
        });
    }

    /// Rewrites the newest block of `kind` in place through
    /// [`Transcript::mutate`]: an answer replacement can grow the painted
    /// rows (markdown table formatting), so it anchors like an append.
    pub(crate) fn reformat_last(&mut self, kind: BlockKind, f: impl FnOnce(&str) -> String) {
        self.mutate(|t| {
            if let Some(block) = t.blocks.iter_mut().rev().find(|b| b.kind == kind) {
                block.text = f(&block.text);
            }
        });
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Deliberately outside [`Transcript::mutate`]: a full reset that zeroes
    /// `scroll_up` and `unseen_rows` itself, so the boundary's baseline and
    /// compensation have nothing left to freeze.
    pub(crate) fn clear(&mut self) {
        self.blocks.clear();
        self.folded.clear();
        self.scroll_up = 0;
        self.unseen_rows = 0;
        self.invalidate_cache();
    }

    pub(crate) fn blocks(&self) -> &[Block] {
        &self.blocks
    }
}

#[cfg(test)]
#[path = "new_activity_tests.rs"]
mod new_activity_tests;
#[cfg(test)]
#[path = "push_tests.rs"]
mod push_tests;
