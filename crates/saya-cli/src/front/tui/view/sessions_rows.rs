//! The sessions picker's pane: the box that names it, the keys it reads, and
//! the list of saved sessions inside it.
//!
//! One pure function — the picker, a width, a height — and the rows to paint.
//! Nothing here reads the terminal or the clock; the rows already carry the age
//! they were listed with. The chrome and the cell arithmetic are
//! [`super::palette_frame`]'s, and what one row says is
//! [`super::sessions_cells`]'s, so this module decides only which rows exist.

use super::composer::{Row, Run};
use super::palette_frame::{MIN_WIDTH, content, enter, frame, sep};
use super::sessions::{FILTER_HINT, NO_MATCH, NO_SESSIONS, Picker, TITLE};
use super::sessions_cells::{Layout, row_runs};
use crate::front::tui::theme::{Charset, Glyph, Token, glyph};

/// The pane at `width` columns and `height` rows, top border first.
///
/// Exactly `height` rows come back: a list shorter than the pane is padded
/// rather than short, and a list longer than it scrolls to keep the selected
/// row on screen.
pub(crate) fn rows(picker: &Picker, width: usize, height: usize) -> Vec<Row> {
    if height == 0 {
        return Vec::new();
    }
    let width = width.max(MIN_WIDTH);
    let charset = picker.charset;
    let listed = picker.listed();
    let visible = height.saturating_sub(2);
    let start = window(picker.selected(), listed.len(), visible);
    let shown: Vec<&_> = listed.iter().skip(start).take(visible).copied().collect();
    let layout = Layout::of(&shown, width.saturating_sub(4), charset);

    let mut out = vec![top(picker, width, charset)];
    if listed.is_empty() {
        out.push(content(vec![empty(picker)], width, charset));
    } else {
        for (offset, row) in shown.iter().enumerate() {
            let selected = start + offset == picker.selected();
            out.push(content(
                row_runs(row, selected, &layout, charset),
                width,
                charset,
            ));
        }
    }
    while out.len() < height - 1 {
        out.push(content(Vec::new(), width, charset));
    }
    out.push(bottom(picker, width, charset));
    // A one-row pane is its top border: there is no room for the second one, and
    // a pane that answered more rows than it was given would spill.
    out.truncate(height);
    out
}

/// The top border: the title, what a press does, and the filter as typed.
fn top(picker: &Picker, width: usize, charset: Charset) -> Row {
    let mark = glyph(Glyph::Rule, charset);
    let mut label = vec![
        Run {
            text: TITLE.to_owned(),
            token: Token::Accent,
        },
        Run {
            text: format!(" {mark} {FILTER_HINT}"),
            token: Token::Dim,
        },
    ];
    if !picker.query.is_empty() {
        label.push(Run {
            text: format!(" {mark} > "),
            token: Token::Dim,
        });
        label.push(Run {
            text: picker.query.clone(),
            token: Token::Fg,
        });
    }
    frame(Glyph::TopLeft, Glyph::TopRight, label, width, charset)
}

/// The bottom border: the ask while the confirm is open, and the keys the pane
/// reads otherwise.
fn bottom(picker: &Picker, width: usize, charset: Charset) -> Row {
    let label = match picker.ask() {
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
                    text: format!(" resume {mark} d delete {mark} Esc close"),
                    token: Token::Dim,
                },
            ]
        }
    };
    frame(Glyph::BottomLeft, Glyph::BottomRight, label, width, charset)
}

/// The row the pane draws when the filter leaves nothing, or when nothing is
/// saved at all.
fn empty(picker: &Picker) -> Run {
    let text = if picker.rows.is_empty() {
        NO_SESSIONS
    } else {
        NO_MATCH
    };
    Run {
        text: text.to_owned(),
        token: Token::Dim,
    }
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
