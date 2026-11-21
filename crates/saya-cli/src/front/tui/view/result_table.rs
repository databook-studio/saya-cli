//! The result's table as the transcript paints it (`02` §3.3): the preview
//! under a turn, and the full view that preview opens.
//!
//! None of the table is drawn twice. The box table, its cells' alignment and
//! its column cap are today's `format_table`; the columns that fit, the
//! wide-table scroller and the state it reads are today's `clip_table_block`;
//! this module paints what those hand back in the transcript's tokens, counts
//! the columns that did not fit, and holds the state the keys move.

use ratatui::text::{Line, Span};

use super::transcript::TranscriptFacts;
use super::turn_rows::{GUTTER, plain};
use crate::front::tui::theme::{Token, style};
use crate::front::view::ResultPreview;
use crate::interactive::tui::{WideTableView, clip_table_block, format_table};

/// How many rows a preview shows: the rest wait behind the full table view.
pub(crate) const PREVIEW_ROWS: usize = 5;

/// The preview's rows: the column names, at most [`PREVIEW_ROWS`] rows, and
/// how many columns the width had to leave out.
pub(crate) fn preview(
    preview: &ResultPreview,
    facts: &TranscriptFacts,
    content: usize,
) -> Vec<Line<'static>> {
    let Some(table) = fitted(preview, &WideTableView::default(), content) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(PREVIEW_ROWS + 2);
    out.push(cells_row(&table.header, facts, GUTTER));
    for row in table.rows.iter().take(PREVIEW_ROWS) {
        out.push(cells_row(row, facts, GUTTER));
    }
    let hidden = table.hidden();
    if hidden > 0 {
        out.push(plain(
            GUTTER,
            &hidden_columns(hidden),
            Token::Dim,
            facts,
            content,
        ));
    }
    out
}

/// The full table view's rows, the visible columns of a table `width` cells
/// wide and the rows a pane of `height` holds.
pub(crate) fn full_rows(
    preview: &ResultPreview,
    view: &TableView,
    facts: &TranscriptFacts,
    width: usize,
    height: usize,
) -> Vec<Line<'static>> {
    let Some(table) = fitted(preview, view.scroller(), width) else {
        return Vec::new();
    };
    table
        .lines
        .iter()
        .take(height)
        .map(|line| match line.starts_with('│') {
            true => cells_row(line, facts, 0),
            false => dim_row(line, facts, 0),
        })
        .collect()
}

/// The full table view's scroll state: where its column window starts, and
/// whether the first column is held while the rest move.
#[derive(Debug, Clone, Default)]
pub(crate) struct TableView {
    scroller: WideTableView,
}

/// The keys `02` §3.3 gives the full table view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Key {
    /// `Ctrl+,` — move the window one column left.
    ColumnLeft,
    /// `Ctrl+.` — move the window one column right.
    ColumnRight,
    /// `Ctrl+P` — hold the first column while the rest scroll.
    Pin,
}

impl TableView {
    /// The state this view hands the scroller.
    fn scroller(&self) -> &WideTableView {
        &self.scroller
    }

    /// The first column the window shows, before the scroller clamps it.
    pub(crate) fn offset(&self) -> usize {
        self.scroller.h_offset
    }

    /// One key's effect on the state, over a table of `columns` columns: the
    /// window moves by a column and stops at the table's edges.
    pub(crate) fn apply(&mut self, key: Key, columns: usize) {
        let last = columns.saturating_sub(1);
        match key {
            Key::ColumnLeft => self.scroller.h_offset = self.scroller.h_offset.saturating_sub(1),
            Key::ColumnRight => {
                self.scroller.h_offset = self.scroller.h_offset.saturating_add(1).min(last);
            }
            Key::Pin => self.scroller.pin_first = !self.scroller.pin_first,
        }
    }
}

/// One table's box, fitted to a width by today's scroller.
struct Fitted {
    /// The clipped box, borders and all: the full table view's rows.
    lines: Vec<String>,
    /// The column names, as the box paints them.
    header: String,
    /// The rows under the names.
    rows: Vec<String>,
    /// How many columns the table holds, and how many of them this width shows.
    columns: usize,
    shown: usize,
}

impl Fitted {
    /// The columns a width could not show.
    fn hidden(&self) -> usize {
        self.columns.saturating_sub(self.shown)
    }
}

/// The table of `preview`'s result, fitted to `width`, or `None` when the
/// result has no box to fit — no columns at all.
fn fitted(preview: &ResultPreview, view: &WideTableView, width: usize) -> Option<Fitted> {
    let block: Vec<String> = format_table(&preview.result)
        .lines()
        .map(str::to_owned)
        .collect();
    // A box table is the ceiling, the column names, the rule, the rows, the
    // floor, and the row count under it; a result with no columns is one line
    // of prose instead.
    if !block.first()?.starts_with('┌') {
        return None;
    }
    let floor = block.len().checked_sub(2)?;
    let lines = clip_table_block(&block, view, width);
    let columns = preview.result.columns.len();
    let header = lines.get(1)?.clone();
    let rows = lines.get(3..floor).unwrap_or_default().to_vec();
    let shown = shown_columns(&lines[0]).min(columns);
    Some(Fitted {
        lines,
        header,
        rows,
        columns,
        shown,
    })
}

/// How many columns a box border shows: one plus the junctions between them.
fn shown_columns(border: &str) -> usize {
    border
        .chars()
        .filter(|ch| matches!(ch, '┬' | '┼' | '┴'))
        .count()
        + 1
}

/// One box row as the transcript paints it: `dim` edges, ordinary cells, and
/// `NULL` in `dim` because it is an absence, not a value. The number columns
/// keep the alignment the box gave them.
fn cells_row(line: &str, facts: &TranscriptFacts, indent: usize) -> Line<'static> {
    let mut spans = vec![Span::raw(" ".repeat(indent))];
    for (index, part) in line.split('│').enumerate() {
        if index > 0 {
            spans.push(Span::styled("│", style(Token::Dim, facts.scheme)));
        }
        let token = match part.trim() {
            "NULL" => Token::Dim,
            _ => Token::Fg,
        };
        spans.push(Span::styled(part.to_owned(), style(token, facts.scheme)));
    }
    Line::from(spans)
}

/// A row that holds no cells — a border, a footer — reads as one `dim` run.
fn dim_row(line: &str, facts: &TranscriptFacts, indent: usize) -> Line<'static> {
    Line::from(vec![
        Span::raw(" ".repeat(indent)),
        Span::styled(line.to_owned(), style(Token::Dim, facts.scheme)),
    ])
}

/// How many columns the preview left out, in the words the spec gives them.
fn hidden_columns(hidden: usize) -> String {
    match hidden {
        1 => "+1 col".to_owned(),
        hidden => format!("+{hidden} cols"),
    }
}
