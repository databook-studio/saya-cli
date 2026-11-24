//! The help sheet (`02` §11): every key the surface answers, filed under one of
//! six groups, and scrolled when the screen cannot hold them all.
//!
//! One pure function — a context, a width, a height and a scroll offset — and
//! the rows a pane paints, the shape the panes beside this one keep. Nothing
//! here reads the terminal, the clock or the model: the context carries only
//! the glyph set.
//!
//! The rows are [`super::help_keys`]'s table, never a list of their own. A
//! sheet that re-typed the key list would drift the first time a key moved, and
//! the operator would read a keyboard the surface no longer has.

use super::composer::{Row, Run};
use super::help_keys::{Group, Shortcut, shortcuts};
use super::palette_frame::{content, fit};
use crate::front::tui::theme::{Charset, Glyph, Token, glyph};
use crate::interactive::tui::wrap::cell_width;

/// The sheet's title, on its top border.
pub(crate) const TITLE: &str = "Help";

/// The narrowest sheet drawn: below this not even its border fits.
const MIN_WIDTH: usize = 12;

/// The rows the sheet asks the frame for by default. A frame gives it whatever
/// the transcript has, and the sheet scrolls its body when that is fewer.
pub(crate) const ROWS: usize = 24;

/// The facts the sheet reads: the glyph set it draws with. The groups are
/// always all listed, so no state ever hides a key the surface answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Context {
    /// Which glyph set the sheet draws with.
    pub(crate) charset: Charset,
}

impl Default for Context {
    fn default() -> Self {
        Self {
            charset: Charset::Utf8,
        }
    }
}

/// The rows the sheet of `context` paints at `width` columns and `height` rows,
/// top border first, starting `scroll` rows into its body.
pub(crate) fn rows_scrolled(
    context: Context,
    width: usize,
    height: usize,
    scroll: usize,
) -> Vec<Row> {
    if height == 0 {
        return Vec::new();
    }
    let width = width.max(MIN_WIDTH);
    let charset = context.charset;
    let body = body(charset, width);
    let visible = height.saturating_sub(2);
    // The far end is clamped to the body: the sheet scrolls over its rows and
    // never past them.
    let start = scroll.min(body.len().saturating_sub(visible));
    let mut out = vec![top(charset, width)];
    out.extend(body.into_iter().skip(start).take(visible));
    while out.len() < height.saturating_sub(1) {
        out.push(content(Vec::new(), width, charset));
    }
    if height >= 2 {
        out.push(bottom(charset, width));
    }
    out.truncate(height);
    out
}

/// The rows the sheet of `context` paints at `width` columns and `height` rows,
/// top border first, from its first body row.
pub(crate) fn rows(context: Context, width: usize, height: usize) -> Vec<Row> {
    rows_scrolled(context, width, height, 0)
}

/// The sheet's body: every group's heading and its key rows, in
/// [`Group::ALL`]'s order.
fn body(charset: Charset, width: usize) -> Vec<Row> {
    let column = column();
    let mut body: Vec<Row> = Vec::new();
    for group in Group::ALL {
        if !body.is_empty() {
            body.push(content(Vec::new(), width, charset));
        }
        body.push(content(
            vec![Run {
                text: group.heading().to_owned(),
                token: Token::Accent,
            }],
            width,
            charset,
        ));
        for shortcut in listed().into_iter().filter(|row| row.group == group) {
            body.push(content(shortcut_row(shortcut, column), width, charset));
        }
    }
    body
}

/// The rows the sheet lists: the whole table.
pub(crate) fn listed() -> Vec<&'static Shortcut> {
    shortcuts().collect()
}

/// One row of the sheet: the presses, lined up in one column, and what they do.
fn shortcut_row(shortcut: &Shortcut, column: usize) -> Vec<Run> {
    let pad = column.saturating_sub(cell_width(shortcut.label));
    vec![
        Run {
            text: format!("  {}{}", shortcut.label, " ".repeat(pad)),
            token: Token::Fg,
        },
        Run {
            text: shortcut.action.to_owned(),
            token: Token::Dim,
        },
    ]
}

/// The key column's width: the widest label the table carries, so every action
/// starts at the same cell.
fn column() -> usize {
    shortcuts()
        .map(|shortcut| cell_width(shortcut.label) + 2)
        .max()
        .unwrap_or(0)
}

/// The top border: the sheet's title.
fn top(charset: Charset, width: usize) -> Row {
    frame(
        Glyph::TopLeft,
        Glyph::TopRight,
        vec![Run {
            text: TITLE.to_owned(),
            token: Token::Accent,
        }],
        width,
        charset,
    )
}

/// The bottom border: the rule that closes the sheet. The keys the sheet lists
/// are its body — no key of its own is named here, because the one that closes
/// it belongs to the runtime (`T14`), not to this pane.
fn bottom(charset: Charset, width: usize) -> Row {
    frame(
        Glyph::BottomLeft,
        Glyph::BottomRight,
        Vec::new(),
        width,
        charset,
    )
}

/// A border row: a corner, the label runs, the rule that fills the rest, and
/// the other corner. Every row is exactly `width` cells.
fn frame(left: Glyph, right: Glyph, label: Vec<Run>, width: usize, charset: Charset) -> Row {
    let room = width.saturating_sub(4);
    let label = fit_runs(label, room);
    let used: usize = label.iter().map(|run| cell_width(&run.text)).sum();
    let rule = glyph(Glyph::Rule, charset);
    let mut runs = vec![Run {
        text: format!("{} ", glyph(left, charset)),
        token: Token::Dim,
    }];
    runs.extend(label);
    runs.push(Run {
        text: format!(
            " {}{}",
            rule.repeat(room.saturating_sub(used)),
            glyph(right, charset)
        ),
        token: Token::Dim,
    });
    Row { runs }
}

/// Fits runs to `room` cells: each run keeps what is left of the room, and the
/// run that runs out is cut with an ellipsis.
fn fit_runs(runs: Vec<Run>, room: usize) -> Vec<Run> {
    let mut left = room;
    let mut out = Vec::new();
    for run in runs {
        if left == 0 {
            break;
        }
        let text = fit(&run.text, left);
        left = left.saturating_sub(cell_width(&text));
        out.push(Run {
            text,
            token: run.token,
        });
    }
    out
}
