//! `02` §9.1's welcome box: the first run, before any provider can answer.
//!
//! A box 56 cells wide, centred horizontally in the pane, holding the news that
//! Saya has nothing to answer with yet and the three keys that get the operator
//! moving. Every row is padded to the box's width so its right edge stays in
//! line, and every glyph comes from the theme, so `--ascii` needs no second
//! path here.

use ratatui::text::{Line, Span};

use super::empty::EmptyFacts;
use super::turn_rows::truncate_to;
use crate::front::tui::theme::{Glyph, Token, glyph, style};
use crate::interactive::tui::wrap::cell_width;

/// `02` §9.1 draws its box 56 cells wide.
const BOX_WIDTH: usize = 56;

/// The air inside the box's edges: two cells each side of the words.
const INSET: usize = 2;

/// The widest a key may be before the column moves: `Enter` is the longest.
const KEY_COLUMN: usize = 5;

/// The three ways forward, as the key and the words beside it.
const KEYS: [(&str, &str); 3] = [
    ("Enter", "set up a provider (2 minutes)"),
    ("d", "try the demo database first"),
    ("q", "quit"),
];

/// `02` §9.1's welcome box, centred in `width`.
pub(super) fn welcome_rows(facts: &EmptyFacts, width: usize) -> Vec<Line<'static>> {
    let box_width = BOX_WIDTH.min(width);
    let left = width.saturating_sub(box_width) / 2;
    let mut rows = vec![border(
        Glyph::TopLeft,
        Glyph::TopRight,
        facts,
        box_width,
        left,
    )];
    rows.push(Line::from(vec![
        Span::raw(" ".repeat(left)),
        Span::styled(
            format!("{}  ", glyph(Glyph::Edge, facts.charset)),
            style(Token::Dim, facts.scheme),
        ),
        Span::styled(
            pad("Welcome to Saya", content(box_width)),
            style(Token::Accent, facts.scheme),
        ),
        Span::styled(
            format!("  {}", glyph(Glyph::Edge, facts.charset)),
            style(Token::Dim, facts.scheme),
        ),
    ]));
    rows.push(body(
        vec![(
            "Saya needs an AI provider before it can answer.".to_owned(),
            Token::Fg,
        )],
        facts,
        box_width,
        left,
    ));
    // The exact next step, both ways: a command to run, or a key to export.
    // The first screen is never a dead end.
    rows.push(body(
        vec![(
            "Next: run `saya setup` to choose a provider,".to_owned(),
            Token::Fg,
        )],
        facts,
        box_width,
        left,
    ));
    rows.push(body(
        vec![(
            "or export OPENAI_API_KEY and run saya again.".to_owned(),
            Token::Fg,
        )],
        facts,
        box_width,
        left,
    ));
    rows.push(body(Vec::new(), facts, box_width, left));
    for (key, words) in KEYS {
        rows.push(body(
            vec![
                (format!("{key:<KEY_COLUMN$}  "), Token::Accent),
                (words.to_owned(), Token::Fg),
            ],
            facts,
            box_width,
            left,
        ));
    }
    rows.push(border(
        Glyph::BottomLeft,
        Glyph::BottomRight,
        facts,
        box_width,
        left,
    ));
    rows
}

/// The cells a body row has for its words, between the edges and the inset.
fn content(box_width: usize) -> usize {
    box_width.saturating_sub(2 * (1 + INSET))
}

/// One body row: the edges, the runs in the order they read, and the air that
/// keeps the right edge in line.
fn body(
    runs: Vec<(String, Token)>,
    facts: &EmptyFacts,
    box_width: usize,
    left: usize,
) -> Line<'static> {
    let room = content(box_width);
    let edge = glyph(Glyph::Edge, facts.charset);
    let mut spans = vec![
        Span::raw(" ".repeat(left)),
        Span::styled(
            format!("{edge}{}", " ".repeat(INSET)),
            style(Token::Dim, facts.scheme),
        ),
    ];
    let mut used = 0;
    for (text, token) in runs {
        let fitted = truncate_to(&text, room.saturating_sub(used));
        used += cell_width(&fitted);
        spans.push(Span::styled(fitted, style(token, facts.scheme)));
    }
    spans.push(Span::styled(
        format!("{}{edge}", " ".repeat(room.saturating_sub(used) + INSET)),
        style(Token::Dim, facts.scheme),
    ));
    Line::from(spans)
}

/// A corner, the rule, and the other corner.
fn border(
    left_corner: Glyph,
    right_corner: Glyph,
    facts: &EmptyFacts,
    box_width: usize,
    left: usize,
) -> Line<'static> {
    Line::from(vec![
        Span::raw(" ".repeat(left)),
        Span::styled(
            glyph(left_corner, facts.charset).to_owned(),
            style(Token::Dim, facts.scheme),
        ),
        Span::styled(
            glyph(Glyph::Rule, facts.charset).repeat(box_width.saturating_sub(2)),
            style(Token::Dim, facts.scheme),
        ),
        Span::styled(
            glyph(right_corner, facts.charset).to_owned(),
            style(Token::Dim, facts.scheme),
        ),
    ])
}

/// Pads `text` to `room` cells: the box's body keeps its right edge in line
/// with every other row.
fn pad(text: &str, room: usize) -> String {
    let fitted = truncate_to(text, room);
    let used = cell_width(&fitted);
    format!("{fitted}{}", " ".repeat(room.saturating_sub(used)))
}
