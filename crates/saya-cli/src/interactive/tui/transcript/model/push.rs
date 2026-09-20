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

    pub(crate) fn push(&mut self, kind: BlockKind, text: impl Into<String>) {
        let chapter = chapters::chapter_for(&self.blocks, kind);
        let text = text.into();
        self.blocks.push(Block {
            kind,
            text,
            chapter,
            group: None,
        });
        self.enforce_bounds();
        self.invalidate_cache();
    }

    pub(crate) fn append_delta(&mut self, kind: BlockKind, delta: &str) {
        if let Some(last) = self.blocks.last_mut().filter(|last| last.kind == kind) {
            last.text.push_str(delta);
            self.enforce_bounds();
            self.invalidate_cache();
        } else {
            self.push(kind, delta);
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
