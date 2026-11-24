//! The focus mark (`02` §6): the `▸` a focused entry carries where its gutter
//! would be, in the `focus` token.
//!
//! The mark takes the block's first two cells and nothing else moves, so the
//! operator sees which block the keyboard is on without the transcript
//! reflowing under them. A block whose first row carries no gutter of its own
//! (an empty row inside an answer) has nothing to mark and is left alone.

use std::borrow::Cow;

use ratatui::text::Line;

use super::transcript::TranscriptFacts;
use super::turn_rows::GUTTER;
use crate::front::tui::theme::{Glyph, Token, glyph, style};
use crate::interactive::tui::wrap::cell_width;

/// Marks `rows` as the focused entry's, or leaves them as they are.
///
/// Only the block's first row carries the mark, and only in cells the block
/// left as gutter: a row that opens with words of its own has no gutter to
/// trade, so it is left alone rather than having its first letters eaten.
pub(super) fn mark(rows: &mut [Line<'static>], focused: bool, facts: &TranscriptFacts) {
    if !focused {
        return;
    }
    let Some(gutter) = rows.first_mut().and_then(|row| row.spans.first_mut()) else {
        return;
    };
    let width = cell_width(gutter.content.as_ref());
    if width < GUTTER || !gutter.content.chars().all(|cell| cell == ' ') {
        return;
    }
    gutter.content = Cow::Owned(format!(
        "{} {}",
        glyph(Glyph::Selected, facts.charset),
        " ".repeat(width - GUTTER)
    ));
    gutter.style = style(Token::Focus, facts.scheme);
}
