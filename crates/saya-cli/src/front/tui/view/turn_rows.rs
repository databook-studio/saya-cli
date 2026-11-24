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

/// `words` cut to `room` cells, marked with an ellipsis when it was cut.
pub(super) fn truncate_to(words: &str, room: usize) -> String {
    if cell_width(words) <= room {
        return words.to_owned();
    }
    match room {
        0 => String::new(),
        room => format!("{}…", truncate_cells(words, room - 1)),
    }
}

/// `words` cut to `room` cells at a word boundary, marked with an ellipsis when
/// it was cut.
///
/// A compact line is cut to the width, and it is cut where a word ends: half a
/// command reads as a command that is not there, and a step line is what an
/// operator reads to tell two calls apart. A line with no whole word inside the
/// room keeps the plain cut — there is nowhere better to stop.
pub(super) fn clip_words(words: &str, room: usize) -> String {
    if cell_width(words) <= room {
        return words.to_owned();
    }
    let cut = match room {
        0 => return String::new(),
        room => truncate_cells(words, room - 1),
    };
    let kept = match cut.rfind(char::is_whitespace) {
        Some(at) if !cut[..at].trim().is_empty() => cut[..at].trim_end().to_owned(),
        _ => cut.trim_end().to_owned(),
    };
    format!("{}…", kept.trim_end())
}

/// A block's mark and its words, wrapped under the mark at `indent`: the first
/// row carries the glyph, and every later row aligns under the words.
pub(super) fn mark_wrapped_at(
    indent: usize,
    mark: (&str, Token),
    words: &str,
    words_token: Token,
    facts: &TranscriptFacts,
    content: usize,
) -> Vec<Line<'static>> {
    let room = content
        .saturating_sub(indent + cell_width(mark.0) + 1)
        .max(1);
    wrapped(words, room)
        .into_iter()
        .enumerate()
        .map(|(index, row)| {
            let (prefix, token) = match index {
                0 => (mark.0.to_owned(), mark.1),
                _ => (" ".repeat(cell_width(mark.0)), words_token),
            };
            Line::from(vec![
                Span::raw(" ".repeat(indent)),
                Span::styled(prefix, style(token, facts.scheme)),
                Span::raw(" "),
                Span::styled(row, style(words_token, facts.scheme)),
            ])
        })
        .collect()
}

/// A block's mark and its words, wrapped under the mark: the first row carries
/// the glyph, and every later row aligns under the words. Nothing is cut with
/// an ellipsis — a reason that wraps is still the whole reason.
pub(super) fn marked_wrapped(
    mark: (&str, Token),
    words: &str,
    words_token: Token,
    facts: &TranscriptFacts,
    content: usize,
) -> Vec<Line<'static>> {
    mark_wrapped_at(GUTTER, mark, words, words_token, facts, content)
}

/// One logical line wrapped to `room` cells, never wider than the room.
pub(super) fn wrapped(text: &str, room: usize) -> Vec<String> {
    let width = room.max(1);
    wrap_cells(text, width)
        .into_iter()
        .map(|row| truncate_cells(&row, width))
        .collect()
}
