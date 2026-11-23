//! The contextual help sheet (`02` §11): the keys that do something in the
//! state the operator is in, and only those.
//!
//! One pure function — a context, a width, a height — and the rows a pane
//! paints, the shape the panes beside this one keep. Nothing here reads the
//! terminal, the clock or the model: [`Context::of`] is the one place that
//! turns the model into the three facts the sheet needs, so a test can draw any
//! state the surface can be in without a key press.
//!
//! The rows are [`super::help_keys`]'s table, never a list of their own. A
//! sheet that re-typed the key list would drift the first time a key moved, and
//! the operator would read a keyboard the surface no longer has.

use super::composer::{Row, Run};
use super::help_keys::{Group, SHORTCUTS, Shortcut, When};
use super::palette_frame::{content, fit};
use crate::front::tui::model::Model;
use crate::front::tui::theme::{Charset, Glyph, Token, glyph};
use crate::interactive::tui::wrap::cell_width;

/// The sheet's title, on its top border.
pub(crate) const TITLE: &str = "Help";

/// The narrowest sheet drawn: below this not even its border fits.
const MIN_WIDTH: usize = 12;

/// The rows the sheet asks the frame for: its two borders, the columns' three
/// headings, and a row for each key an idle surface answers.
pub(crate) const ROWS: usize = 17;

/// The facts the sheet reads: which state the surface is in, and the glyph set
/// it draws with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Context {
    /// Whether a card is open and waiting to be answered.
    pub(crate) card: bool,
    /// Whether that card offered a token a session grant could record.
    pub(crate) grant: bool,
    /// Whether a line is held behind the running turn.
    pub(crate) queued: bool,
    /// Whether a turn is running.
    pub(crate) busy: bool,
    /// Which glyph set the sheet draws with.
    pub(crate) charset: Charset,
}

impl Context {
    /// The context `model` is in.
    pub(crate) fn of(model: &Model) -> Self {
        Self {
            card: model.card().is_some(),
            grant: model.card().is_some_and(|card| card.grant.is_some()),
            queued: model.queued().is_some(),
            busy: model.is_busy(),
            charset: Charset::Utf8,
        }
    }
}

impl Default for Context {
    fn default() -> Self {
        Self {
            card: false,
            grant: false,
            queued: false,
            busy: false,
            charset: Charset::Utf8,
        }
    }
}

/// The rows the sheet of `context` paints at `width` columns and `height` rows,
/// top border first.
///
/// Exactly `height` rows come back: a sheet with fewer keys than the screen has
/// rows is padded rather than short, and a sheet with more is cut at the second
/// border, because the border is where the sheet ends.
pub(crate) fn rows(context: Context, width: usize, height: usize) -> Vec<Row> {
    if height == 0 {
        return Vec::new();
    }
    let width = width.max(MIN_WIDTH);
    let charset = context.charset;
    let shortcuts = listed(&context);
    let column = column(&shortcuts);
    let mut body: Vec<Row> = Vec::new();
    for group in Group::ALL {
        let members: Vec<&Shortcut> = shortcuts
            .iter()
            .copied()
            .filter(|shortcut| shortcut.group == group)
            .collect();
        if members.is_empty() {
            continue;
        }
        // A blank row between columns keeps the three headings apart when the
        // sheet is drawn as one column.
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
        for shortcut in members {
            body.push(content(shortcut_row(shortcut, column), width, charset));
        }
    }
    let mut out = vec![top(charset, width)];
    out.extend(body.into_iter().take(height.saturating_sub(2)));
    while out.len() < height - 1 {
        out.push(content(Vec::new(), width, charset));
    }
    out.push(bottom(charset, width));
    // A one-row sheet is its top border: there is no room for the second one,
    // and a pane that answered more rows than it was given would spill.
    out.truncate(height);
    out
}

/// The rows the sheet lists in `context`: the table, filtered by the state each
/// row is valid in (`02` §11).
pub(crate) fn listed(context: &Context) -> Vec<&'static Shortcut> {
    SHORTCUTS
        .iter()
        .filter(|shortcut| shows(shortcut.when, context))
        .collect()
}

/// Whether a row valid in `when` belongs in `context`.
///
/// An open card owns the keyboard — `keys.rs` says so — so the composer's keys
/// are not listed while one waits, and Esc is listed once: as the card's
/// denial, never as the turn's stop.
fn shows(when: When, context: &Context) -> bool {
    match when {
        When::Card => context.card,
        When::CardGrant => context.card && context.grant,
        When::Composer => !context.card,
        When::Queued => !context.card && context.queued,
        When::Busy => !context.card && context.busy,
    }
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

/// The key column's width: the widest label the listed rows carry, so every
/// action starts at the same cell.
fn column(shortcuts: &[&Shortcut]) -> usize {
    shortcuts
        .iter()
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
