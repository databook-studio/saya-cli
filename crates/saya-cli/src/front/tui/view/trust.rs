//! The workspace trust question, as the pane it is painted in (`02` §9.4).
//!
//! The words are the trust module's own ([`BODY`], [`NOTE`]): the new surface
//! asks the same question in the same sentences the old modal asks, and only
//! the answers row is this pane's — `y` and `n`, the two keys this surface
//! reads, each with the sentence it means. The box and its cell arithmetic are
//! [`super::palette_frame`]'s, reused rather than copied.

use super::composer::{Row, Run};
use super::palette_frame::{MIN_WIDTH, content, frame};
use crate::front::tui::direct_trust::{BODY, NOTE};
use crate::front::tui::theme::{Charset, Glyph, Token};
use crate::interactive::tui::wrap::wrap_cells;

/// The pane's title: today's modal's own title row.
pub(crate) const TITLE: &str = " trust this folder? ";

/// The answers row: the two keys this pane reads, and what each one means.
pub(crate) const ANSWERS: &str = "[y] trust this folder    [n] continue unbound";

/// The narrowest pane the question is asked in: the answers row, one row, and
/// the borders.
pub(crate) const WIDTH: usize = ANSWERS.len() + 4;

/// The rows the pane is drawn in: the question, the answers row, the note, and
/// the two borders — never more than the modal's own ceiling.
pub(crate) const ROWS: usize = 8;

/// The pane at `width` columns and `height` rows, top border first.
///
/// Exactly `height` rows come back: the question wrapped to the pane's inner
/// width, the answers row, the session-only note, and the two borders — padded
/// rather than short when there is room to spare, and cut at its height when
/// there is not.
pub(crate) fn rows(width: usize, height: usize, charset: Charset) -> Vec<Row> {
    if height == 0 {
        return Vec::new();
    }
    let width = width.max(MIN_WIDTH);
    let mut out = vec![top(width, charset)];
    out.extend(question(width, charset));
    out.push(content(Vec::new(), width, charset));
    out.push(content(
        vec![Run {
            text: ANSWERS.to_owned(),
            token: Token::Warn,
        }],
        width,
        charset,
    ));
    out.push(content(
        vec![Run {
            text: NOTE.to_owned(),
            token: Token::Dim,
        }],
        width,
        charset,
    ));
    while out.len() < height - 1 {
        out.push(content(Vec::new(), width, charset));
    }
    out.push(bottom(width, charset));
    out.truncate(height);
    out
}

/// The question's words, wrapped to the pane's inner width.
fn question(width: usize, charset: Charset) -> Vec<Row> {
    let room = width.saturating_sub(4);
    wrap_cells(BODY, room)
        .into_iter()
        .map(|line| {
            content(
                vec![Run {
                    text: line,
                    token: Token::Fg,
                }],
                width,
                charset,
            )
        })
        .collect()
}

/// The top border: the pane's title.
fn top(width: usize, charset: Charset) -> Row {
    frame(
        Glyph::TopLeft,
        Glyph::TopRight,
        vec![Run {
            text: TITLE.to_owned(),
            token: Token::Warn,
        }],
        width,
        charset,
    )
}

/// The bottom border: nothing but the box's own edge — the answers are a row of
/// their own inside it.
fn bottom(width: usize, charset: Charset) -> Row {
    frame(
        Glyph::BottomLeft,
        Glyph::BottomRight,
        Vec::new(),
        width,
        charset,
    )
}
