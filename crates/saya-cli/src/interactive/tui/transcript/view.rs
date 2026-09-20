use std::rc::Rc;

use super::rows::{Row, WrappedLines, wrap_word_aware};
use super::{BlockKind, Transcript};

impl Transcript {
    pub(super) fn lines(&self, width: usize) -> Rc<WrappedLines> {
        let eff = width.max(1);
        if let Some((_, lines)) = self.cache.borrow().as_ref().filter(|(w, _)| *w == eff) {
            return Rc::clone(lines);
        }
        let mut lines = Vec::new();
        let mut skip_until = 0;
        for (i, block) in self.blocks.iter().enumerate() {
            // A folded chapter paints one summary row for its whole range:
            // the opener's block emits it, every later block skips. The row
            // re-checks `foldable` (so an evicted opener or a chapter that
            // became live again unfolds itself) and counts as one row, like
            // a collapsed tool group.
            if i < skip_until {
                continue;
            }
            if let Some((start, end)) = super::chapters::chapter_range(&self.blocks, block.chapter)
                && start == i
                && self.is_folded(block.chapter)
                && let Some(row) = super::chapters::folded_row(&self.blocks, block.chapter, eff)
            {
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
        let rc = Rc::new(lines);
        *self.cache.borrow_mut() = Some((eff, Rc::clone(&rc)));
        rc
    }

    pub(crate) fn wrapped(&self, width: usize) -> WrappedLines {
        self.lines(width)
            .iter()
            .map(|row| Row {
                kind: row.kind,
                text: row.text.clone(),
                is_label: row.is_label,
            })
            .collect()
    }

    pub(crate) fn total_lines(&self, width: usize) -> usize {
        self.lines(width).len()
    }

    pub(crate) fn view(&self, width: usize, height: usize) -> WrappedLines {
        if height == 0 {
            return Vec::new();
        }
        // Every row `lines()` produces is a row that paints — label rows
        // included — so the tail view windows over all rows and
        // `total_lines` equals the painted row count again.
        let painted: WrappedLines = self
            .lines(width)
            .iter()
            .map(|row| Row {
                kind: row.kind,
                text: row.text.clone(),
                is_label: row.is_label,
            })
            .collect();
        let rem = painted.len().saturating_sub(height);
        if rem == 0 {
            return painted;
        }
        let start = rem - self.scroll_up.min(rem);
        painted[start..start + height].to_vec()
    }

    /// Like [`view`], but table blocks are painted through the wide-table
    /// view: each table line is horizontally clipped (and optionally
    /// column-filtered) to `width` instead of left whole. The line count is
    /// unchanged, so vertical scroll metrics from [`total_lines`] still match.
    /// The offset/column state is passed in from the view — it never lives on
    /// the transcript data.
    pub(crate) fn wide_view(
        &self,
        width: usize,
        height: usize,
        wv: &super::super::types::WideTableView,
    ) -> WrappedLines {
        let src = self.lines(width);
        let mut full: WrappedLines = Vec::with_capacity(src.len());
        let mut i = 0;
        while i < src.len() {
            if src[i].kind == BlockKind::Table && !src[i].is_label {
                // A table block's lines are contiguous; collect the run, then
                // split it into individual tables (each starts with ┌) so two
                // adjacent results are clipped independently. The label row
                // opens the run and passes through untouched.
                let run_start = i;
                while i < src.len() && src[i].kind == BlockKind::Table {
                    i += 1;
                }
                let run = src[run_start..i]
                    .iter()
                    .map(|row| row.text.clone())
                    .collect::<Vec<_>>();
                // A run of exactly one line is the lone label row (a
                // non-empty table block is label + grid lines): it passes
                // through untouched, never through the grid clipper.
                if run.len() == 1 && src[run_start].is_label {
                    full.push(Row {
                        kind: BlockKind::Table,
                        text: run[0].clone(),
                        is_label: true,
                    });
                    continue;
                }
                // The run opens with the label row, ahead of the grid lines
                // the clipper expects — clip the grid, keep the label
                // verbatim, and the line count is unchanged.
                let (label, grid) = match src[run_start].is_label {
                    true => (Some(&src[run_start]), &run[1..]),
                    false => (None, &run[..]),
                };
                let clipped = super::super::table::clip_table_block(grid, wv, width);
                debug_assert_eq!(clipped.len(), grid.len());
                if let Some(label) = label {
                    full.push(Row {
                        kind: BlockKind::Table,
                        text: label.text.clone(),
                        is_label: true,
                    });
                }
                for line in clipped {
                    full.push(Row::body(BlockKind::Table, line));
                }
            } else {
                full.push(Row {
                    kind: src[i].kind,
                    text: src[i].text.clone(),
                    is_label: src[i].is_label,
                });
                i += 1;
            }
        }
        // Every row `lines()` produces is a row that paints — label rows
        // included — so the tail view windows over the full rows and
        // `total_lines` equals the painted row count again.
        let out: WrappedLines = full;
        if height == 0 {
            return Vec::new();
        }
        let rem = out.len().saturating_sub(height);
        if rem == 0 {
            return out;
        }
        let start = rem - self.scroll_up.min(rem);
        out[start..start + height].to_vec()
    }
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod view_tests;
