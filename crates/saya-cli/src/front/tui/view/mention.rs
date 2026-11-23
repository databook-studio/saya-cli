//! The `@` list's pane: the workspace paths the composer is offering, painted
//! above the composer box.
//!
//! The rows are the model's ([`crate::front::tui::model_mention`]), which is
//! where the filtering lives; this module only lays them out — a title row that
//! names what the list is, one row per path with a mark on the selected one, and
//! a footer naming the keys. Every glyph comes from the theme, so `--ascii` needs
//! no second path, and every row is cut to the width it was given rather than
//! allowed to spill over an edge.

use crate::front::tui::model::Model;
use crate::front::tui::theme::{Charset, Glyph, Token, glyph};
use crate::interactive::tui::wrap::{cell_width, truncate_cells};

use super::composer::{Row, Run};

/// The list's title, on its first row.
pub(crate) const TITLE: &str = "Files";

/// What the list says while the walk that fills it is still running.
pub(crate) const FINDING: &str = "looking for files…";
pub(crate) const FINDING_ASCII: &str = "looking for files...";

/// What it says when the workspace offers nothing for what is typed.
pub(crate) const NO_MATCH: &str = "no file matches";

/// The footer: the keys the open list reads.
pub(crate) const HINT: &str = "Tab insert · ↑↓ choose · Esc close";
pub(crate) const HINT_ASCII: &str = "Tab insert | up/down choose | Esc close";

/// The most path rows the pane paints; a longer list is walked with the arrows.
pub(crate) const MAX_VISIBLE: usize = 6;

/// The narrowest pane: below this the list is painted without its frame, one
/// path per row, so a narrow terminal still shows what it is offering.
const MIN_WIDTH: usize = 12;

/// The list's rows, top border first, or nothing when no list is open.
pub(crate) fn rows(model: &Model, width: usize, charset: Charset) -> Vec<Row> {
    let Some(list) = model.mention() else {
        return Vec::new();
    };
    let offered = model.mention_rows();
    let mut out = vec![title_row(list.waiting, width, charset)];
    if offered.is_empty() {
        let words = match list.waiting {
            true => match charset {
                Charset::Utf8 => FINDING,
                Charset::Ascii => FINDING_ASCII,
            },
            false => NO_MATCH,
        };
        out.push(body_row(
            &[Run {
                text: words.to_owned(),
                token: Token::Dim,
            }],
            width,
            charset,
        ));
    }
    let selected = list.selected.min(offered.len().saturating_sub(1));
    for (index, path) in offered.iter().take(MAX_VISIBLE).enumerate() {
        let mark = match index == selected {
            true => format!("{} ", glyph(Glyph::Selected, charset)),
            false => "  ".to_owned(),
        };
        out.push(body_row(
            &[
                Run {
                    text: mark,
                    token: Token::Accent,
                },
                Run {
                    text: path.clone(),
                    token: match index == selected {
                        true => Token::Focus,
                        false => Token::Fg,
                    },
                },
            ],
            width,
            charset,
        ));
    }
    out.push(footer_row(width, charset));
    out
}

/// The list's own height: what [`rows`] returns, without laying it out twice.
pub(crate) fn height(model: &Model) -> usize {
    if model.mention().is_none() {
        return 0;
    }
    let offered = model.mention_rows().len().min(MAX_VISIBLE);
    // The title, the footer, and the rows — or the one row that says why there
    // are none.
    offered.max(1) + 2
}

/// The title row: the list's name, and whether the walk is still running.
fn title_row(waiting: bool, width: usize, charset: Charset) -> Row {
    let mut runs = vec![Run {
        text: TITLE.to_owned(),
        token: Token::Accent,
    }];
    if waiting {
        runs.push(Run {
            text: format!(" {}", glyph(Glyph::Rule, charset)),
            token: Token::Dim,
        });
    }
    framed(Glyph::TopLeft, Glyph::TopRight, &runs, width, charset)
}

/// The footer row: the keys the list reads.
fn footer_row(width: usize, charset: Charset) -> Row {
    let hint = match charset {
        Charset::Utf8 => HINT,
        Charset::Ascii => HINT_ASCII,
    };
    framed(
        Glyph::BottomLeft,
        Glyph::BottomRight,
        &[Run {
            text: hint.to_owned(),
            token: Token::Dim,
        }],
        width,
        charset,
    )
}

/// One path row inside the frame.
fn body_row(runs: &[Run], width: usize, charset: Charset) -> Row {
    framed(Glyph::Edge, Glyph::Edge, runs, width, charset)
}

/// A framed row: the two edge glyphs, the runs between them, padded with spaces
/// so the right edge sits under the right edge above it. A pane narrower than
/// [`MIN_WIDTH`] drops the frame and paints the words: the list is what the
/// operator is reading, not the box around it.
fn framed(left: Glyph, right: Glyph, runs: &[Run], width: usize, charset: Charset) -> Row {
    if width < MIN_WIDTH {
        let text: String = runs.iter().map(|run| run.text.as_str()).collect();
        return Row {
            runs: vec![Run {
                text: truncate_cells(text.trim(), width),
                token: Token::Fg,
            }],
        };
    }
    // Two edges, one space on each side of them, and the room the words have.
    let room = width.saturating_sub(4);
    let mut cut = Vec::new();
    let mut spent = 0usize;
    for run in runs {
        if spent >= room {
            break;
        }
        let text = truncate_cells(&run.text, room - spent);
        spent += cell_width(&text);
        cut.push(Run {
            text,
            token: run.token,
        });
    }
    cut.push(Run {
        text: " ".repeat(room - spent),
        token: Token::Dim,
    });
    let mut row = vec![Run {
        text: format!("{} ", glyph(left, charset)),
        token: Token::Dim,
    }];
    row.extend(cut);
    row.push(Run {
        text: format!(" {}", glyph(right, charset)),
        token: Token::Dim,
    });
    Row { runs: row }
}
