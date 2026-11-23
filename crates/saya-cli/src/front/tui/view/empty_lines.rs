//! The rows the three ready states of `02` §9 share: the header that names the
//! session, the sentence that offers examples, one row per example, and the
//! footer that names the keys.
//!
//! One cell arithmetic for all of them: a two-cell gutter, today's cell-aware
//! wrap, and an ellipsis where a header has more to say than the pane has room
//! for — so no state can paint past its own edge.

use ratatui::text::{Line, Span};

use super::empty::{Database, EmptyFacts};
use super::turn_rows::{GUTTER, truncate_to, wrapped};
use crate::front::tui::theme::{Charset, Token, style};
use crate::interactive::tui::wrap::cell_width;

/// How many examples `02` §9.3 draws under its intro.
pub(super) const EXAMPLES_SHOWN: usize = 3;

/// Where an example's marker sits: one indent in from the header.
const EXAMPLE_INDENT: usize = 4;

/// `02` §9.2's examples: with no database there is no cached schema to ask
/// about, so the spec draws questions about the project the session is in.
/// Every one names a real top-level entry; a session with no workspace (or an
/// empty one) is offered the neutral examples instead.
fn workspace_examples(facts: &EmptyFacts) -> Vec<String> {
    match facts.suggestions.is_empty() {
        true => super::empty_suggestions::neutral(),
        false => facts
            .suggestions
            .iter()
            .take(EXAMPLES_SHOWN)
            .cloned()
            .collect(),
    }
}

/// `02` §9.2's footer: the way in to a database, and the keys beside it.
const CONNECT_FOOTER: &str = "/connect to add a database · ? keys · Ctrl+K commands";

/// `02` §9.2: ready, inside a project, with no database.
pub(super) fn no_database_rows(facts: &EmptyFacts, width: usize) -> Vec<Line<'static>> {
    let mut rows = vec![
        header(facts, &session(facts, "no database", Token::Warn), width),
        Line::default(),
    ];
    rows.extend(sentence(
        "Describe a change or ask about this code. For example:",
        Token::Fg,
        facts,
        width,
    ));
    for question in workspace_examples(facts) {
        rows.extend(example(&question, facts, width));
    }
    rows.push(Line::default());
    rows.extend(sentence(CONNECT_FOOTER, Token::Dim, facts, width));
    rows
}

/// `02` §9.3: ready, with a database connected and its schema's questions.
///
/// The questions are the caller's, taken from the first [`EXAMPLES_SHOWN`]: a
/// schema that suggests none leaves the invitation to ask and drops the
/// promise of examples.
pub(super) fn connected_rows(facts: &EmptyFacts, width: usize) -> Vec<Line<'static>> {
    let database = facts.database.clone().unwrap_or_default();
    let questions: Vec<&String> = facts
        .starter_questions
        .iter()
        .take(EXAMPLES_SHOWN)
        .collect();
    let intro = match questions.is_empty() {
        true => format!("Ask about {} or describe a change.", database.name),
        false => format!(
            "Ask about {} or describe a change. For example:",
            database.name
        ),
    };
    let mut rows = vec![
        header(
            facts,
            &session(facts, &label(&database), Token::Info),
            width,
        ),
        Line::default(),
    ];
    rows.extend(sentence(&intro, Token::Fg, facts, width));
    for question in questions {
        rows.extend(example(question, facts, width));
    }
    rows
}

/// `02` §9.4: ready, with no workspace bound — the one line says what is off.
pub(super) fn no_workspace_rows(facts: &EmptyFacts, width: usize) -> Vec<Line<'static>> {
    let database = match &facts.database {
        Some(database) => (database.name.clone(), Token::Info),
        None => ("no database".to_owned(), Token::Warn),
    };
    let mut rows = vec![
        header(
            facts,
            &[
                ("Saya".to_owned(), Token::Accent),
                (
                    "no workspace (outside a git repository)".to_owned(),
                    Token::Warn,
                ),
                database,
            ],
            width,
        ),
        Line::default(),
    ];
    rows.extend(sentence(
        "File tools are off here. Start Saya inside a project to edit code.",
        Token::Fg,
        facts,
        width,
    ));
    rows
}

/// The database's word in the header: its name, its dialect and its size.
fn label(database: &Database) -> String {
    let tables = match database.tables {
        1 => "table".to_owned(),
        count => format!("{count} tables"),
    };
    format!("{} ({}, {tables})", database.name, database.dialect)
}

/// The header's three parts: the product, the project and the database.
fn session(facts: &EmptyFacts, database: &str, token: Token) -> Vec<(String, Token)> {
    vec![
        ("Saya".to_owned(), Token::Accent),
        (
            facts
                .workspace
                .clone()
                .unwrap_or_else(|| "no workspace".to_owned()),
            Token::Dim,
        ),
        (database.to_owned(), token),
    ]
}

/// The header row: the parts joined by the spec's middle dot, cut to the pane
/// rather than painted past it.
fn header(facts: &EmptyFacts, parts: &[(String, Token)], width: usize) -> Line<'static> {
    const SEP: &str = " · ";
    let mut spans = vec![Span::raw(" ".repeat(GUTTER))];
    let mut room = width.saturating_sub(GUTTER);
    for (index, (text, token)) in parts.iter().enumerate() {
        if index > 0 {
            if room < cell_width(SEP) {
                break;
            }
            spans.push(Span::styled(SEP, style(Token::Dim, facts.scheme)));
            room -= cell_width(SEP);
        }
        let fitted = truncate_to(text, room);
        room -= cell_width(&fitted);
        spans.push(Span::styled(fitted, style(*token, facts.scheme)));
        if room == 0 {
            break;
        }
    }
    Line::from(spans)
}

/// A sentence under the gutter, wrapped to the pane and never cut.
fn sentence(text: &str, token: Token, facts: &EmptyFacts, width: usize) -> Vec<Line<'static>> {
    let room = width.saturating_sub(GUTTER).max(1);
    wrapped(text, room)
        .into_iter()
        .map(|row| {
            Line::from(vec![
                Span::raw(" ".repeat(GUTTER)),
                Span::styled(row, style(token, facts.scheme)),
            ])
        })
        .collect()
}

/// One example question: the marker on its first row, the marker's width as
/// air on the rows it wraps onto.
fn example(question: &str, facts: &EmptyFacts, width: usize) -> Vec<Line<'static>> {
    let mark = marker(facts.charset);
    let room = width
        .saturating_sub(EXAMPLE_INDENT + cell_width(mark) + 1)
        .max(1);
    wrapped(question, room)
        .into_iter()
        .enumerate()
        .map(|(index, row)| {
            let prefix = match index {
                0 => mark.to_owned(),
                _ => " ".repeat(cell_width(mark)),
            };
            Line::from(vec![
                Span::raw(" ".repeat(EXAMPLE_INDENT)),
                Span::styled(prefix, style(Token::Dim, facts.scheme)),
                Span::raw(" "),
                Span::styled(row, style(Token::Fg, facts.scheme)),
            ])
        })
        .collect()
}

/// The examples' marker: `02` §9 draws `›`, which the glyph table of `02` §11
/// does not name — so it keeps `❯`'s ASCII fallback, `>`.
fn marker(charset: Charset) -> &'static str {
    match charset {
        Charset::Utf8 => "›",
        Charset::Ascii => ">",
    }
}
