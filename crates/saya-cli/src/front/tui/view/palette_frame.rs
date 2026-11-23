//! The palette's frame: the border that names it, the border that lists its
//! keys, and the two row shapes every other row is measured against.
//!
//! Only geometry lives here — the list, the usage line and the window over them
//! are [`super::palette_rows`]'s. Every row this module hands back is exactly
//! the width it was given: the frame's own cells come off first, and a label or
//! a run that does not fit is cut with an ellipsis rather than allowed to spill
//! over an edge.

use super::composer::{Row, Run};
use super::palette::{Palette, TITLE};
use crate::front::tui::theme::{Charset, Glyph, Token, glyph};
use crate::interactive::tui::wrap::{cell_width, truncate_cells};

/// The narrowest pane the palette draws: below this not even its frame fits.
pub(super) const MIN_WIDTH: usize = 12;

/// The separator the key row and an alias note read with. The glyph table
/// names the transcript's glyphs; this one carries its ASCII form here, as the
/// status line's caveat mark does.
const SEP: &str = "·";
const SEP_ASCII: &str = "-";

/// The Enter mark the key row leads with.
const ENTER: &str = "↵";
const ENTER_ASCII: &str = "[enter]";

/// The query mark on the top border (`02` §11's `─ > exp`).
const ASK: &str = "> ";

/// The top border: the title, and the query when there is one.
pub(super) fn top(palette: &Palette, width: usize) -> Row {
    let mut label = vec![Run {
        text: String::from(TITLE),
        token: Token::Accent,
    }];
    if !palette.query.is_empty() {
        let mark = glyph(Glyph::Rule, palette.charset);
        label.push(Run {
            text: format!(" {mark} {ASK}"),
            token: Token::Dim,
        });
        label.push(Run {
            text: palette.query.clone(),
            token: Token::Fg,
        });
    }
    frame(
        Glyph::TopLeft,
        Glyph::TopRight,
        label,
        width,
        palette.charset,
    )
}

/// The bottom border: the keys that drive the pane.
pub(super) fn bottom(charset: Charset, width: usize) -> Row {
    let mark = sep(charset);
    frame(
        Glyph::BottomLeft,
        Glyph::BottomRight,
        vec![
            Run {
                text: enter(charset).to_owned(),
                token: Token::Dim,
            },
            Run {
                text: format!(" insert {mark} Tab complete {mark} Esc close"),
                token: Token::Dim,
            },
        ],
        width,
        charset,
    )
}

/// A content row: the edges, the runs, and the padding that keeps the right
/// edge in line.
pub(super) fn content(runs: Vec<Run>, width: usize, charset: Charset) -> Row {
    let room = width.saturating_sub(4);
    let runs = fit_runs(runs, room);
    let used: usize = runs.iter().map(|run| cell_width(&run.text)).sum();
    let edge = glyph(Glyph::Edge, charset);
    let mut out = vec![Run {
        text: format!("{edge} "),
        token: Token::Dim,
    }];
    out.extend(runs);
    out.push(Run {
        text: format!("{} {edge}", " ".repeat(room.saturating_sub(used))),
        token: Token::Fg,
    });
    Row { runs: out }
}

/// `text` cut to `room` cells at a word boundary, marked with an ellipsis when
/// it had to be cut.
pub(super) fn fit_words(text: &str, room: usize) -> String {
    if cell_width(text) <= room {
        return text.to_owned();
    }
    if room == 0 {
        return String::new();
    }
    let mut cut = truncate_cells(text, room - 1);
    if let Some(space) = cut.rfind(' ') {
        let whole = cut[..space].trim_end();
        if !whole.is_empty() {
            cut = whole.to_owned();
        }
    }
    format!("{cut}…")
}

/// `text` cut to `room` cells, marked with an ellipsis when it had to be cut.
pub(super) fn fit(text: &str, room: usize) -> String {
    if cell_width(text) <= room {
        return text.to_owned();
    }
    if room == 0 {
        return String::new();
    }
    format!("{}…", truncate_cells(text, room - 1))
}

/// The separator, in the glyph set in force.
pub(super) const fn sep(charset: Charset) -> &'static str {
    match charset {
        Charset::Utf8 => SEP,
        Charset::Ascii => SEP_ASCII,
    }
}

/// The Enter mark, in the glyph set in force.
pub(super) const fn enter(charset: Charset) -> &'static str {
    match charset {
        Charset::Utf8 => ENTER,
        Charset::Ascii => ENTER_ASCII,
    }
}

/// A border row: a corner, the label runs, the rule that fills the rest, and
/// the other corner. Shared with the panes beside this one, so a border row has
/// one definition rather than one per pane.
pub(super) fn frame(
    left: Glyph,
    right: Glyph,
    label: Vec<Run>,
    width: usize,
    charset: Charset,
) -> Row {
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
