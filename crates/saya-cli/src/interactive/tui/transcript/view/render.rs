//! The one place a block becomes painted rows — and, for navigation, the row
//! index each block's output begins at.

use super::super::chapters;
use super::super::rows::{Row, WrappedLines, wrap_word_aware};
use super::super::{BlockKind, Transcript};

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
            if let Some((start, end)) = chapters::chapter_range(&self.blocks, block.chapter)
                && start == i
                && self.is_folded(block.chapter)
                && let Some(row) = chapters::folded_row(&self.blocks, block.chapter, eff)
            {
                fold_start = lines.len();
                lines.push(row);
                skip_until = end;
                continue;
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
