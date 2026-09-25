//! The one place a block becomes painted rows — and, for navigation, the row
//! index each block's output begins at.

use super::super::chapters;
use super::super::rows::{Row, WrappedLines, wrap_word_aware};
use super::super::{Block, BlockKind, Transcript};

impl Transcript {
    /// The rendered rows plus the row index each block's output begins at, in
    /// `blocks()` order. A folded chapter collapses its members to the summary
    /// row's index, so a result inside a fold still has an address to step to.
    /// Never cached: the caller re-derives after any fold change.
    pub(crate) fn render(&self, width: usize) -> (WrappedLines, Vec<usize>) {
        let eff = width.max(1);
        let mut lines = Vec::new();
        let mut starts: Vec<usize> = Vec::with_capacity(self.blocks.len());
        let mut skip_until = 0;
        let mut fold_start = 0;
        for (i, block) in self.blocks.iter().enumerate() {
            if i < skip_until {
                starts.push(fold_start);
                continue;
            }
            starts.push(lines.len());
            // A folded chapter paints one summary row for its whole range:
            // the opener's block emits it, every later block skips. The row
            // re-checks `foldable` (so an evicted opener or a chapter that
            // became live again unfolds itself) and counts as one row, like
            // a collapsed tool group.
            //
            // `is_folded` gates the whole branch: an unfolded block pays no
            // range discovery at all. Chapters are contiguous and only
            // advance, so a folded run starts exactly where the previous
            // block carries a different chapter — no search — and ends at
            // the first differing chapter after it: one scan over the run,
            // once per folded chapter, never per visited block (audit F08:
            // the old branch scanned the slice twice per visited block
            // before even asking whether it was folded).
            if self.is_folded(block.chapter)
                && (i == 0 || {
                    #[cfg(test)]
                    range_probe::note();
                    self.blocks[i - 1].chapter != block.chapter
                })
            {
                let end = chapter_run_end(&self.blocks, i);
                if let Some(row) = chapters::folded_row(&self.blocks, block.chapter, i, end, eff) {
                    fold_start = lines.len();
                    lines.push(row);
                    skip_until = end;
                    continue;
                }
            }
            if block.text.is_empty()
                && block
                    .group
                    .as_ref()
                    .filter(|group| group.expanded)
                    .is_none()
            {
                // Spacers (`push_spacer`'s `(System, "")`) stay visible but
                // bare: one empty body row, no label row — a label above every
                // blank line would be noise, and empty rows paint blank.
                lines.push(Row::body(block.kind, String::new()));
                continue;
            }
            // One label row per non-empty block, ahead of its body rows — so
            // every scroll/find metric derived from `lines()` counts the row
            // that paints, keeping "one entry per painted row" true.
            let mut labelled_yet = false;
            let mut emit_body = |raw: &str, lines: &mut WrappedLines| {
                if raw.is_empty() {
                    // Blank lines inside a block stay bare.
                    lines.push(Row::body(block.kind, String::new()));
                    return;
                }
                if !labelled_yet {
                    labelled_yet = true;
                    if let Some(label) = Row::label(block.kind) {
                        lines.push(label);
                    }
                }
                if block.kind == BlockKind::Table {
                    // A table row is one line of box drawing; word-wrapping it
                    // destroys the grid, so each line is kept whole and the
                    // view clips it horizontally instead.
                    lines.push(Row::body(block.kind, raw.to_string()));
                } else {
                    wrap_word_aware(raw, eff, block.kind, lines);
                }
            };
            if let Some(group) = block.group.as_ref().filter(|group| group.expanded) {
                for raw in std::iter::once(group.open_header.as_str())
                    .chain(group.detail.iter().map(String::as_str))
                {
                    emit_body(raw, &mut lines);
                }
                continue;
            }
            for raw in block.text.split('\n') {
                emit_body(raw, &mut lines);
            }
        }
        (lines, starts)
    }
}

/// One past the run of blocks carrying `blocks[start].chapter`: chapters are
/// contiguous and only advance (see [`chapters::chapter_range`]), so the run
/// ends at the first differing chapter — a forward scan over the run alone,
/// paid once per folded run instead of per visited block. `pub(super)` so
/// the scale tests can price the one-pass discovery against the old
/// whole-slice scans.
pub(super) fn chapter_run_end(blocks: &[Block], start: usize) -> usize {
    blocks[start + 1..]
        .iter()
        .position(|b| {
            #[cfg(test)]
            range_probe::note();
            b.chapter != blocks[start].chapter
        })
        .map_or(blocks.len(), |rel| start + 1 + rel)
}

/// Benchmark seam for the scale tests: counts the block-chapter comparisons
/// range discovery performs while rendering. `cfg(test)` only — the notes
/// compile out of production builds entirely.
#[cfg(test)]
pub(crate) mod range_probe {
    use std::cell::Cell;

    thread_local! {
        static COMPARISONS: Cell<usize> = const { Cell::new(0) };
    }

    pub(crate) fn note() {
        COMPARISONS.with(|count| count.set(count.get() + 1));
    }

    pub(crate) fn reset() {
        COMPARISONS.with(|count| count.set(0));
    }

    pub(crate) fn take() -> usize {
        COMPARISONS.with(|count| count.replace(0))
    }
}
