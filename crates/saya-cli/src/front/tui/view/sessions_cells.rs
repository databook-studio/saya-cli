//! One row of the sessions picker as cells: the selection mark, the state mark,
//! the title, and the facts beside it.
//!
//! A row reads `▸ ✓ <title>  <database>  <turns>  <age>`. Each column is as wide
//! as its visible rows need, so the facts line up down the pane, and a column
//! that does not fit gives way whole: the title is what an operator chooses by,
//! so it is the last thing cut.

use super::composer::Run;
use super::palette_frame::fit;
use super::sessions_row::{SessionRow, State};
use crate::front::tui::theme::{Charset, Glyph, Token, glyph};
use crate::interactive::tui::wrap::cell_width;

/// The mark of a record with no run. The theme's glyph table carries the marks
/// the transcript draws; this one is the picker's, so it is named here beside
/// the ASCII form that replaces it.
const RECORD: &str = "◌";
const RECORD_ASCII: &str = "o";

/// What a row says for a database it has none of.
const NO_DATABASE: &str = "—";
const NO_DATABASE_ASCII: &str = "-";

/// The tag the session this process is on carries.
const THIS_SESSION: &str = "(this session)";

/// The words a read-only record ends with.
const READ_ONLY: &str = "read-only";

/// The cells between two of a row's columns.
const GAP: usize = 2;

/// The narrowest title column worth keeping: below it a row says nothing an
/// operator can choose by, so the columns beside it go instead.
const MIN_TITLE: usize = 12;

/// The column widths one pane draws: the mark, the title left over, and the
/// four facts beside it. A width of zero is a column that gave way.
pub(super) struct Layout {
    /// The widest state mark, so the titles line up.
    mark: usize,
    title: usize,
    database: usize,
    turns: usize,
    age: usize,
    note: usize,
}

impl Layout {
    /// The layout `shown` rows need in `room` cells.
    ///
    /// Every column starts at the widest row and then gives way, least telling
    /// first: a record's `read-only` is already carried by its mark, the age is
    /// the order the store listed it in, the turn count and the database say
    /// what the session was, and the title says which one it is.
    pub(super) fn of(shown: &[&SessionRow], room: usize, charset: Charset) -> Self {
        let mark = shown
            .iter()
            .map(|row| cell_width(mark(row.state, charset)))
            .max()
            .unwrap_or(1);
        let mut layout = Self {
            mark,
            title: 0,
            database: widest(shown, |row| database(row, charset)),
            turns: widest(shown, |row| turns(row.turns)),
            age: widest(shown, age),
            note: widest(shown, note),
        };
        // The selection cell and the mark lead every row, then a cell of gap.
        let lead = mark + 3;
        while layout.title_room(room, lead) < MIN_TITLE {
            if layout.note > 0 {
                layout.note = 0;
            } else if layout.age > 0 {
                layout.age = 0;
            } else if layout.turns > 0 {
                layout.turns = 0;
            } else if layout.database > 0 {
                layout.database = 0;
            } else {
                break;
            }
        }
        layout.title = layout.title_room(room, lead);
        layout
    }

    /// What the title has once the lead and the columns this layout keeps are
    /// taken out of `room`.
    fn title_room(&self, room: usize, lead: usize) -> usize {
        room.saturating_sub(lead + self.tail())
    }

    /// The cells every column beside the title takes, gaps included.
    fn tail(&self) -> usize {
        [self.database, self.turns, self.age, self.note]
            .into_iter()
            .map(|width| if width == 0 { 0 } else { GAP + width })
            .sum()
    }
}

/// One list row: the selection mark, the state mark, the title, and the facts
/// the columns keep.
pub(super) fn row_runs(
    row: &SessionRow,
    selected: bool,
    layout: &Layout,
    charset: Charset,
) -> Vec<Run> {
    let token = if selected { Token::Focus } else { Token::Fg };
    let marker = if selected {
        glyph(Glyph::Selected, charset)
    } else {
        " "
    };
    let state_mark = mark(row.state, charset);
    let title = fit(&row.title, layout.title);
    let mut runs = vec![
        Run {
            text: format!(
                "{marker} {state_mark}{} ",
                " ".repeat(layout.mark - cell_width(state_mark))
            ),
            token,
        },
        Run {
            text: format!("{title}{}", " ".repeat(layout.title - cell_width(&title))),
            token,
        },
    ];
    for (text, width) in [
        (database(row, charset), layout.database),
        (turns(row.turns), layout.turns),
        (age(row), layout.age),
        (note(row), layout.note),
    ] {
        if width == 0 {
            continue;
        }
        runs.push(Run {
            text: format!(
                "{}{text}{}",
                " ".repeat(GAP),
                " ".repeat(width - cell_width(&text))
            ),
            token: Token::Dim,
        });
    }
    runs
}

/// The widest of `text` over `shown`.
fn widest(shown: &[&SessionRow], text: impl Fn(&SessionRow) -> String) -> usize {
    shown
        .iter()
        .map(|row| cell_width(&text(row)))
        .max()
        .unwrap_or(0)
}

/// The mark of a row's state, in the glyph set in force.
fn mark(state: State, charset: Charset) -> &'static str {
    match (state, charset) {
        (State::Record, Charset::Utf8) => RECORD,
        (State::Record, Charset::Ascii) => RECORD_ASCII,
        (State::Open, charset) => glyph(Glyph::Bullet, charset),
        (State::Finished, charset) => glyph(Glyph::Check, charset),
    }
}

/// The database a row was on, as the column says it.
fn database(row: &SessionRow, charset: Charset) -> String {
    match row.database.as_deref() {
        Some(database) => database.to_owned(),
        None => match charset {
            Charset::Utf8 => NO_DATABASE.to_owned(),
            Charset::Ascii => NO_DATABASE_ASCII.to_owned(),
        },
    }
}

/// How many turns a row settled.
fn turns(turns: usize) -> String {
    match turns {
        1 => String::from("1 turn"),
        turns => format!("{turns} turns"),
    }
}

/// How long ago the row was saved, and the tag the session in hand carries.
fn age(row: &SessionRow) -> String {
    if row.current() {
        format!("{} {THIS_SESSION}", row.age)
    } else {
        row.age.clone()
    }
}

/// The words a read-only record ends with, and nothing for any other row.
fn note(row: &SessionRow) -> String {
    if row.read_only {
        READ_ONLY.to_owned()
    } else {
        String::new()
    }
}
