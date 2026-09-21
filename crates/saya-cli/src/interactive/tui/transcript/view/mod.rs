pub(crate) mod render;
pub(crate) mod results;
pub(crate) mod rows;
pub(crate) mod scroll;

use std::rc::Rc;

use super::rows::{Row, WrappedLines};
use super::{BlockKind, Transcript};

impl Transcript {
    pub(super) fn lines(&self, width: usize) -> Rc<WrappedLines> {
        let eff = width.max(1);
        if let Some((_, lines)) = self.cache.borrow().as_ref().filter(|(w, _)| *w == eff) {
            return Rc::clone(lines);
        }
        let (rows, _) = self.render(eff);
        let rc = Rc::new(rows);
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

    /// The newest open pending tool call's facts for the status bar: the one
    /// place "what is running" is already recorded, so the bar reads it here
    /// instead of keeping a second copy beside `RequestState::activity`.
    /// `None` when nothing is open — the bar falls back to the bare tool
    /// name, and the transcript buffer itself is untouched.
    pub(crate) fn newest_open_tool(&self) -> Option<(&str, &serde_json::Value)> {
        self.pending_tools
            .iter()
            .rev()
            .find(|call| call.open)
            .map(|call| (call.name.as_str(), &call.arguments))
    }

    /// Debug seam: how many tool calls are still open. The status-bar test
    /// asserts through this that the fixture really holds an open call, so a
    /// green assertion cannot hide an empty buffer.
    #[cfg(test)]
    pub(crate) fn open_tool_count(&self) -> usize {
        self.pending_tools.iter().filter(|call| call.open).count()
    }
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod view_tests;

#[cfg(test)]
#[path = "rows_unicode_tests.rs"]
mod rows_unicode_tests;
