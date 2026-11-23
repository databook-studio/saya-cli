//! The changes pane (UI5): the files the last turn that wrote changed, each with
//! the diff of it, as the box the operator scrolls.
//!
//! One pure function — the turns, the pane's scroll, a width and a height — and
//! the rows to paint, the shape [`super::jobs_rows`] keeps. The pane holds no
//! diff of its own: the rows are [`crate::front::view::change`]'s, which are
//! [`crate::change_diff`]'s — the same engine and the same bounds the approval
//! panel reads, so a file cannot read one way while it is being approved and
//! another way after it landed.
//!
//! The rows scroll: the pane opens at the first file and `↑↓`/`PgUp`/`PgDn` walk
//! it, keeping the top row it was asked for and never past the end.

use super::composer::{Row, Run};
use super::palette_frame::{MIN_WIDTH, content, frame};
use crate::front::tui::theme::{Charset, Glyph, Token, glyph};
use crate::front::view::{Change, last_changed};

/// The pane's title, on its top border.
pub(crate) const TITLE: &str = "Changed";

/// How many rows the pane asks the frame for: a title, a border, and the first
/// file with its counts.
pub(crate) const ROWS: usize = 12;

/// What the pane says when nothing has been written yet: it is opened by a key
/// that is offered only when a turn changed a file, so this is said on the frame
/// an older run left, never on a live one.
pub(crate) const NOTHING: &str = "no turn of this session has changed a file";

/// Where the pane is scrolled to: its own half of the model, so the rows are
/// read at paint time and a turn that lands while it is open is never stale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Changes {
    /// The first body row painted.
    pub(crate) scroll: usize,
    /// Which glyph set the pane draws with, as every pane carries its own.
    pub(crate) charset: Charset,
}

impl Default for Changes {
    fn default() -> Self {
        Self {
            scroll: 0,
            charset: Charset::Utf8,
        }
    }
}

/// The changes pane for `turns`, scrolled to `changes`, at `width` columns and
/// `height` rows.
pub(crate) fn rows(
    turns: &[crate::front::view::TurnView],
    changes: &Changes,
    width: usize,
    height: usize,
    charset: Charset,
) -> Vec<Row> {
    if height == 0 {
        return Vec::new();
    }
    let width = width.max(MIN_WIDTH);
    let body = body(last_changed(turns), width, charset);
    let visible = height.saturating_sub(2);
    let start = changes.scroll.min(body.len().saturating_sub(1));
    let mut out = vec![top(width, charset)];
    out.extend(body.into_iter().skip(start).take(visible));
    while out.len() < height - 1 {
        out.push(content(Vec::new(), width, charset));
    }
    out.push(bottom(width, charset));
    out.truncate(height);
    out
}

/// The pane's rows before the window: one header per file — its path and both
/// counts — then the rows of its diff, signed exactly as the panel signed them.
fn body(files: Vec<Change>, width: usize, charset: Charset) -> Vec<Row> {
    if files.is_empty() {
        return vec![content(
            vec![Run {
                text: String::from(NOTHING),
                token: Token::Dim,
            }],
            width,
            charset,
        )];
    }
    let mut rows = Vec::new();
    for file in files {
        rows.push(content(
            vec![
                Run {
                    text: file.path,
                    token: Token::Accent,
                },
                Run {
                    text: format!("  [+{} −{}]", file.added, file.removed),
                    token: Token::Dim,
                },
            ],
            width,
            charset,
        ));
        rows.extend(
            file.rows
                .into_iter()
                .map(|row| content(vec![signed(row)], width, charset)),
        );
    }
    rows
}

/// One row of a diff, in the meaning its sign states: a line the change writes,
/// a line it takes away, where the hunk sits, or context.
fn signed(row: String) -> Run {
    let token = match row.chars().next() {
        Some('+') => Token::Ok,
        Some('-') => Token::Bad,
        Some('@') | Some('\\') => Token::Dim,
        _ => Token::Fg,
    };
    Run { text: row, token }
}

/// The top border: the pane's title.
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

/// The bottom border: the keys the pane reads. `Esc` is the one key every pane
/// closes on, so it is named here and read in `overlay_keys`.
fn bottom(width: usize, charset: Charset) -> Row {
    let up = glyph(Glyph::Selected, charset);
    frame(
        Glyph::BottomLeft,
        Glyph::BottomRight,
        vec![Run {
            text: format!("{up}{up} scroll · PgUp/PgDn page · Esc close"),
            token: Token::Dim,
        }],
        width,
        charset,
    )
}
