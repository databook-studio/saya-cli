//! The palette's rows (`02` §11): the list the query matched, the usage line of
//! the selected row, and the window that keeps that row on screen.
//!
//! One pure function — the palette, a width, a height — and the rows to paint.
//! Nothing here reads the terminal or the clock. The chrome and the cell
//! arithmetic are [`super::palette_frame`]'s, so this module only decides what
//! each row says.

use super::composer::{Row, Run};
use super::palette::{NO_MATCH, Palette, USAGE};
use super::palette_frame::{MIN_WIDTH, bottom, content, fit, sep, top};
use super::palette_match::Entry;
use crate::front::tui::theme::{Charset, Glyph, Token, glyph};
use crate::interactive::tui::wrap::cell_width;

/// The pane at `width` columns and `height` rows, top border first.
///
/// The chrome takes three rows — the top border, the usage row and the bottom
/// border — and the list gets what is left. The list scrolls to keep the
/// selected row on screen, because the catalogue is longer than any terminal.
pub(crate) fn rows(palette: &Palette, width: usize, height: usize) -> Vec<Row> {
    let width = width.max(MIN_WIDTH);
    let charset = palette.charset;
    let room = width.saturating_sub(4);
    let entries = palette.entries();
    let visible = height.saturating_sub(3);
    let mut body = Vec::new();
    if entries.is_empty() {
        body.push(unmatched(palette.query()));
    } else {
        let start = window(palette.selected(), entries.len(), visible);
        let column = name_column(&entries, start, visible);
        for (offset, entry) in entries.iter().enumerate().skip(start).take(visible) {
            let selected = offset == palette.selected();
            body.push(entry_row(entry, selected, column, room, charset));
        }
    }
    let drawn = body.len();
    let mut out = vec![top(palette, width)];
    for runs in body {
        out.push(content(runs, width, charset));
    }
    for _ in drawn..visible {
        out.push(content(Vec::new(), width, charset));
    }
    out.push(usage(palette, width, charset));
    out.push(bottom(charset, width));
    out
}

/// The list row of one command: the selection mark, the canonical name, the
/// one-line summary and the heading it is filed under, right-aligned.
///
/// A row that matched through an alias says so in its summary, because the name
/// it shows is the command's own — an operator who types `/approvals` is shown
/// `/mode`, and the row has to say why.
fn entry_row(
    entry: &Entry,
    selected: bool,
    column: usize,
    room: usize,
    charset: Charset,
) -> Vec<Run> {
    let token = if selected { Token::Focus } else { Token::Fg };
    let marker = if selected {
        glyph(Glyph::Selected, charset)
    } else {
        " "
    };
    let name = fit(&format!("/{}", entry.name), column);
    let pad = column.saturating_sub(cell_width(&name));
    let heading = entry.category.heading();
    // The mark, the name column, its two-cell gap, and the gap and heading on
    // the right are fixed; the summary gets what they leave.
    let fixed = 2 + column + 2;
    let tail = cell_width(heading) + 2;
    let summary = fit(
        &described(entry, charset),
        room.saturating_sub(fixed + tail),
    );
    let used = fixed + cell_width(&summary);
    let mut runs = vec![
        Run {
            text: format!("{marker} {name}{}  ", " ".repeat(pad)),
            token,
        },
        Run {
            text: summary,
            token: Token::Dim,
        },
    ];
    // The heading gives way before the summary does: a row without its
    // summary says nothing an operator can choose by, while the heading only
    // says where the command is filed.
    if used + 1 + cell_width(heading) <= room {
        runs.push(Run {
            text: format!(
                "{}{heading}",
                " ".repeat(room.saturating_sub(used + cell_width(heading)))
            ),
            token: Token::Dim,
        });
    }
    runs
}

/// The row's summary: the catalogue's one line, and the alias that found it
/// when the canonical name did not.
fn described(entry: &Entry, charset: Charset) -> String {
    match entry.alias {
        Some(alias) => format!("via /{alias} {} {}", sep(charset), entry.summary),
        None => entry.summary.to_owned(),
    }
}

/// The empty row: what the pane says when the query matches nothing.
fn unmatched(query: &str) -> Vec<Run> {
    vec![Run {
        text: format!("{NO_MATCH} \"{query}\""),
        token: Token::Dim,
    }]
}

/// The usage row: the selected command's synopsis, or nothing when the query
/// matched no command.
fn usage(palette: &Palette, width: usize, charset: Charset) -> Row {
    match palette.selected_entry() {
        Some(entry) => content(
            vec![
                Run {
                    text: format!("{USAGE} "),
                    token: Token::Dim,
                },
                Run {
                    text: entry.usage.to_owned(),
                    token: Token::Fg,
                },
            ],
            width,
            charset,
        ),
        None => content(Vec::new(), width, charset),
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

/// The widest name the visible rows hold: one column, so the summaries line up.
fn name_column(entries: &[Entry], start: usize, visible: usize) -> usize {
    entries
        .iter()
        .skip(start)
        .take(visible)
        .map(|entry| cell_width(entry.name) + 1)
        .max()
        .unwrap_or(0)
}
