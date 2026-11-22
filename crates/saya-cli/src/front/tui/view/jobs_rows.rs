//! The jobs drawer's pane: the box that names it, the rows of jobs inside it,
//! and the keys it reads (`02` §8).
//!
//! One pure function — the drawer, a width, a height — and the rows to paint.
//! Nothing here reads the terminal or the clock: a row already carries its age
//! and its last summary. The chrome and the cell arithmetic are
//! [`super::palette_frame`]'s, so this module decides only which rows exist and
//! where the drawer sits: at 100 columns or more it takes the transcript's
//! right-hand side, and below that it is a bottom sheet across the width.
//! [`placement`] is the one decision, and the runtime sizes the area from it.

use super::composer::{Row, Run};
use super::jobs::{Drawer, Job, NO_JOBS, TITLE};
use super::palette_frame::{MIN_WIDTH, content, enter, frame, sep};
use crate::front::tui::theme::{Charset, Glyph, Token, glyph};

/// The width at which the drawer moves from a bottom sheet to the transcript's
/// right-hand side (`02` §8: "right-side over the transcript at ≥ 100 cols,
/// bottom sheet at 80").
pub(crate) const SIDE_FROM: usize = 100;

/// How wide the side pane is, when the screen is wide enough for one: `02` §8's
/// box, and never more than half the screen.
pub(crate) const SIDE_WIDTH: usize = 48;

/// How many rows the bottom sheet takes, title and key row included.
pub(crate) const SHEET_ROWS: usize = 6;

/// Where the drawer sits on the frame it is painted on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Placement {
    /// Over the transcript's right-hand side, from 100 columns up.
    Side,
    /// A sheet across the bottom of the transcript, below 100 columns.
    Bottom,
}

/// Which of the two the drawer at `width` columns takes (`02` §8).
pub(crate) const fn placement(width: usize) -> Placement {
    if width >= SIDE_FROM {
        Placement::Side
    } else {
        Placement::Bottom
    }
}

/// The columns a side pane takes out of `width`: `02` §8's box, never more than
/// half the screen and never narrower than a box needs.
pub(crate) fn side_width(width: usize) -> usize {
    let half = width / 2;
    if SIDE_WIDTH < half { SIDE_WIDTH } else { half }.max(MIN_WIDTH)
}

/// The drawer at `width` columns and `height` rows, top border first.
///
/// Exactly `height` rows come back: a list shorter than the pane is padded
/// rather than short, and a list longer than it scrolls to keep the selected
/// row on screen.
pub(crate) fn rows(drawer: &Drawer, width: usize, height: usize, charset: Charset) -> Vec<Row> {
    if height == 0 {
        return Vec::new();
    }
    let width = width.max(MIN_WIDTH);
    let visible = height.saturating_sub(2);
    let start = window(drawer.selected(), drawer.rows.len(), visible);

    let mut out = vec![top(width, charset)];
    if drawer.rows.is_empty() {
        out.push(content(vec![empty()], width, charset));
    } else {
        for (offset, row) in drawer.rows.iter().skip(start).take(visible).enumerate() {
            let selected = start + offset == drawer.selected();
            out.push(content(cells(row, selected, charset), width, charset));
        }
    }
    while out.len() < height - 1 {
        out.push(content(Vec::new(), width, charset));
    }
    out.push(bottom(drawer, width, charset));
    // A one-row pane is its top border: there is no room for the second one, and
    // a pane that answered more rows than it was given would spill.
    out.truncate(height);
    out
}

/// One job's row: the selection mark, the state's mark, the id, and the facts
/// `02` §8 draws beside them.
fn cells(row: &Job, selected: bool, charset: Charset) -> Vec<Run> {
    let token = if selected { Token::Focus } else { Token::Fg };
    let marker = if selected {
        glyph(Glyph::Selected, charset)
    } else {
        " "
    };
    vec![
        Run {
            text: format!("{marker} {} {}", glyph(row.state.mark(), charset), row.id),
            token,
        },
        Run {
            text: format!("  {}", row.facts()),
            token: Token::Dim,
        },
    ]
}

/// The first list row drawn: the selection kept on screen, and never past the
/// end of the list.
fn window(selected: usize, len: usize, visible: usize) -> usize {
    if visible == 0 {
        return 0;
    }
    selected
        .saturating_sub(visible - 1)
        .min(len.saturating_sub(visible))
}

/// The top border: the title, and the drop note when the list does not fit.
fn top(width: usize, charset: Charset) -> Row {
    frame(
        Glyph::TopLeft,
        Glyph::TopRight,
        vec![Run {
            text: String::from(TITLE),
            token: Token::Accent,
        }],
        width,
        charset,
    )
}

/// The bottom border: the ask while the confirm is open, and the keys the pane
/// reads otherwise.
fn bottom(drawer: &Drawer, width: usize, charset: Charset) -> Row {
    let label = match drawer.ask() {
        Some(ask) => vec![Run {
            text: ask,
            token: Token::Warn,
        }],
        None => {
            let mark = sep(charset);
            vec![
                Run {
                    text: enter(charset).to_owned(),
                    token: Token::Dim,
                },
                Run {
                    text: format!(" open {mark} a approve {mark} x cancel {mark} Esc close"),
                    token: Token::Dim,
                },
            ]
        }
    };
    frame(Glyph::BottomLeft, Glyph::BottomRight, label, width, charset)
}

/// The row a drawer with no jobs paints.
fn empty() -> Run {
    Run {
        text: String::from(NO_JOBS),
        token: Token::Dim,
    }
}
