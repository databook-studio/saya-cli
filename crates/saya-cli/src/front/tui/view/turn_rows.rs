//! The rows every block of a turn is painted from: the gutter, the glyph and
//! its words, one run of words, and the wrap they share.
//!
//! The cell arithmetic lives here once, so no block can paint wider than the
//! pane it is painted in and every block starts at the same column.

use ratatui::text::{Line, Span};

use super::transcript::TranscriptFacts;
use crate::front::tui::theme::{Token, style};
use crate::interactive::tui::wrap::{cell_width, truncate_cells, wrap_cells};

/// The two cells every block starts after: the transcript's left gutter.
pub(super) const GUTTER: usize = 2;

/// Where a step's own words start: under the block's summary line.
pub(super) const STEP_INDENT: usize = 4;

/// A row under the block gutter: its glyph, the words beside it, the tokens
/// each paints — and never more cells than the pane has room for.
pub(super) fn marked(
    indent: usize,
    mark: &str,
    words: &str,
    mark_token: Token,
    words_token: Token,
    facts: &TranscriptFacts,
    content: usize,
) -> Line<'static> {
    let room = content.saturating_sub(indent + cell_width(mark) + 1);
    Line::from(vec![
        Span::raw(" ".repeat(indent)),
        Span::styled(mark.to_owned(), style(mark_token, facts.scheme)),
        Span::raw(" "),
        Span::styled(
            truncate_cells(words, room),
            style(words_token, facts.scheme),
        ),
    ])
}

/// A row of one run of words under the block gutter.
pub(super) fn plain(
    indent: usize,
    words: &str,
    token: Token,
    facts: &TranscriptFacts,
    content: usize,
) -> Line<'static> {
    let room = content.saturating_sub(indent);
    Line::from(vec![
        Span::raw(" ".repeat(indent)),
        Span::styled(truncate_cells(words, room), style(token, facts.scheme)),
    ])
}

/// A row of spans another renderer made — the answer body's markdown — set in
/// under the gutter. An empty line stays an empty row rather than carrying the
/// gutter's spaces.
pub(super) fn indented(spans: Vec<Span<'static>>) -> Line<'static> {
    let words: String = spans.iter().map(|span| span.content.as_ref()).collect();
    if words.trim().is_empty() {
        return Line::default();
    }
    let mut all = vec![Span::raw(" ".repeat(GUTTER))];
    all.extend(spans);
    Line::from(all)
}

/// One logical line wrapped to `room` cells, never wider than the room.
pub(super) fn wrapped(text: &str, room: usize) -> Vec<String> {
    let width = room.max(1);
    wrap_cells(text, width)
        .into_iter()
        .map(|row| truncate_cells(&row, width))
        .collect()
}
