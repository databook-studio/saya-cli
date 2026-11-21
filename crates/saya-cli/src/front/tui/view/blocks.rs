//! The turn's work blocks (`02` §3.4-§3.6) and the session's own line
//! (§3.10): an edit and its diff, a command and how it ended, a helper's work,
//! and one compact dim line for what the session has to say.
//!
//! Every one is a pure painter: the entry's facts and the content column in,
//! the rows out. What ends a turn — the error and the stop — is
//! [`super::blocks_stop`]; the model's question is [`super::blocks_question`].

use std::path::Path;
use std::time::Duration;

use ratatui::text::{Line, Span};

use super::transcript::TranscriptFacts;
use super::turn_rows::{GUTTER, STEP_INDENT, plain, truncate_to};
use crate::front::tui::theme::{Glyph, Token, glyph, style};
use crate::front::view::{
    Command, CommandState, DiffKind, DiffLine, Edit, Helper, HelperState, Notice, command_words,
};
use crate::interactive::tui::wrap::{cell_width, truncate_cells};

/// The diff rows an edit block shows before it counts the rest: `02` §3.4 draws
/// three and names no other count, so three rows and a `+N more lines` row are
/// the compact diff, and Enter opens the whole one.
const DIFF_ROWS: usize = 3;

/// The air a right-aligned key keeps between the words and itself.
const AIR: usize = 2;

/// An edit: its file and counts, then the diff's rows (`02` §3.4).
pub(super) fn edit_rows(
    edit: &Edit,
    facts: &TranscriptFacts,
    content: usize,
) -> Vec<Line<'static>> {
    let title = format!("Edit {}", edit.path);
    let added = format!("+{}", edit.added());
    let removed = format!("−{}", edit.removed());
    let hint = format!("{} Enter", glyph(Glyph::Selected, facts.charset));
    let mut out = vec![header(
        (glyph(Glyph::Edit, facts.charset), Token::Accent),
        &[
            (&title, Token::Fg),
            ("  ", Token::Fg),
            (&added, Token::Ok),
            (" ", Token::Fg),
            (&removed, Token::Bad),
        ],
        &[(&hint, Token::Dim)],
        facts,
        content,
    )];
    let numbers = edit
        .lines
        .iter()
        .filter_map(|line| line.number)
        .map(digits)
        .max()
        .unwrap_or(1);
    let shown = DIFF_ROWS.min(edit.lines.len());
    for line in &edit.lines[..shown] {
        out.push(diff_row(line, numbers, facts, content));
    }
    let withheld = edit.lines.len() - shown;
    if withheld > 0 {
        let more = format!("+{withheld} more {}", plural(withheld, "line", "lines"));
        out.push(plain(
            GUTTER + STEP_INDENT,
            &more,
            Token::Dim,
            facts,
            content,
        ));
    }
    out
}

/// One diff row: its line number, the gutter that says what changed, then the
/// line itself. The `+`/`-` gutter carries the meaning with the colour, never
/// instead of it (`02` §3.4).
fn diff_row(
    line: &DiffLine,
    numbers: usize,
    facts: &TranscriptFacts,
    content: usize,
) -> Line<'static> {
    let (sign, token) = match line.kind {
        DiffKind::Context => (" ", Token::Fg),
        DiffKind::Added => ("+", Token::Ok),
        DiffKind::Removed => ("-", Token::Bad),
    };
    let number = match line.number {
        Some(number) => format!("{number:>numbers$}"),
        None => " ".repeat(numbers),
    };
    let room = content.saturating_sub(GUTTER + STEP_INDENT + numbers + 3);
    Line::from(vec![
        Span::raw(" ".repeat(GUTTER + STEP_INDENT)),
        Span::styled(number, style(Token::Dim, facts.scheme)),
        Span::raw(" "),
        Span::styled(
            glyph(Glyph::Edge, facts.charset).to_owned(),
            style(Token::Dim, facts.scheme),
        ),
        Span::styled(sign.to_owned(), style(token, facts.scheme)),
        Span::styled(truncate_cells(&line.text, room), style(token, facts.scheme)),
    ])
}

/// A command the turn ran: `$ the command`, and its ending and duration
/// right-aligned (`02` §3.5).
pub(super) fn command_rows(
    command: &Command,
    facts: &TranscriptFacts,
    content: usize,
) -> Vec<Line<'static>> {
    let words = command_words(
        Some(&command.command),
        facts.workspace.as_deref().map(Path::new),
    )
    .unwrap_or(command.command.as_str())
    .to_owned();
    let (mark, token, ending) = match &command.state {
        CommandState::Finished { elapsed } => (
            Glyph::Check,
            Token::Ok,
            format!("finished · {}", duration_text(*elapsed)),
        ),
        CommandState::Failed { reason, elapsed } => (
            Glyph::Cross,
            Token::Bad,
            format!("{reason} · {}", duration_text(*elapsed)),
        ),
    };
    let ending = format!("{} {ending}", glyph(mark, facts.charset));
    vec![header(
        ("$", Token::Dim),
        &[(&words, Token::Code)],
        &[(&ending, token)],
        facts,
        content,
    )]
}

/// A helper's work as one line: its id, its brief in its own words, its steps
/// and how it ended (`02` §3.6).
pub(super) fn helper_rows(
    helper: &Helper,
    facts: &TranscriptFacts,
    content: usize,
) -> Vec<Line<'static>> {
    let (mark, token, word) = match helper.state {
        HelperState::Running => (Glyph::Bullet, Token::Ok, "running"),
        HelperState::Done => (Glyph::Check, Token::Ok, "done"),
        HelperState::Failed => (Glyph::Cross, Token::Bad, "failed"),
    };
    let steps = format!("{} {}", helper.steps, plural(helper.steps, "step", "steps"));
    let head = format!(
        "helper {} · \"{}\" · {steps} · ",
        helper.id, helper.question
    );
    let ending = format!("{} {word}", glyph(mark, facts.charset));
    let hint = format!("{} Enter", glyph(Glyph::Selected, facts.charset));
    vec![header(
        (glyph(Glyph::Hint, facts.charset), Token::Accent),
        &[(&head, Token::Fg), (&ending, token)],
        &[(&hint, Token::Dim)],
        facts,
        content,
    )]
}

/// Something the session itself has to say: one compact dim line (`02` §3.10).
pub(super) fn system_line(
    notice: &Notice,
    facts: &TranscriptFacts,
    content: usize,
) -> Vec<Line<'static>> {
    vec![plain(GUTTER, &notice.message, Token::Dim, facts, content)]
}

/// A block's header: its mark, the run of words beside it, each with its own
/// token, and, when one is given, the key that opens the block right-aligned
/// against the content column's edge.
fn header(
    mark: (&str, Token),
    words: &[(&str, Token)],
    hint: &[(&str, Token)],
    facts: &TranscriptFacts,
    content: usize,
) -> Line<'static> {
    let hint_width: usize = hint.iter().map(|(text, _)| cell_width(text)).sum();
    let air_min = match hint.is_empty() {
        true => 0,
        false => AIR,
    };
    let room = content.saturating_sub(cell_width(mark.0) + 1 + air_min + hint_width);
    let mut spans = vec![
        Span::raw(" ".repeat(GUTTER)),
        Span::styled(mark.0.to_owned(), style(mark.1, facts.scheme)),
        Span::raw(" "),
    ];
    let mut left = room;
    for (text, token) in words {
        // Every run gives way to the room the header has left, so a narrow pane
        // cuts the words with an ellipsis instead of painting past its edge.
        let words = truncate_to(text, left);
        left = left.saturating_sub(cell_width(&words));
        spans.push(Span::styled(words, style(*token, facts.scheme)));
    }
    if !hint.is_empty() {
        let used = cell_width(mark.0) + 1 + (room - left);
        let air = content.saturating_sub(used + hint_width).max(AIR);
        spans.push(Span::raw(" ".repeat(air)));
        for (text, token) in hint {
            spans.push(Span::styled(
                (*text).to_owned(),
                style(*token, facts.scheme),
            ));
        }
    }
    Line::from(spans)
}

/// How long a command took: tenths under a whole second, seconds above it.
fn duration_text(elapsed: Duration) -> String {
    let tenths = elapsed.as_millis() / 100;
    match tenths % 10 {
        0 => format!("{}s", tenths / 10),
        part => format!("{}.{part}s", tenths / 10),
    }
}

/// How many digits a line number needs.
fn digits(number: u32) -> usize {
    number.to_string().len()
}

/// `one` or `many` for `count`.
fn plural<'a>(count: usize, one: &'a str, many: &'a str) -> &'a str {
    match count {
        1 => one,
        _ => many,
    }
}
