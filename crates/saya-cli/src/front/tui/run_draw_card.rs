//! The approval panel: the call the operator has to answer, drawn as the box the
//! frame paints directly above the composer.
//!
//! The words are [`super::view::cards`]'s — the title, the call's own bytes,
//! where it runs and the answers it offers. This module says how many rows that
//! is and paints them inside a rule, so the grid can reserve the panel's rows
//! before anything is painted and the two can never disagree about its height.
//!
//! The panel is the surface's one loud pane: a rule all the way across the
//! content column, its title in the focus token, and the answers one per line.
//! What it gives up on a short screen is body, never answers: an operator must
//! always be able to see how to answer.

use ratatui::text::{Line, Span};

use super::model::Model;
use super::run_draw::Facts;
use super::theme::{Charset, Glyph, Token, glyph, style};
use super::view::{Ink, approval_waiting, fit, pad};

/// The token a body row is painted with, by the meaning the card gave it: a
/// line the change adds is `ok`, one it takes away is `bad`, and a hunk header
/// or the count of what is withheld is dim — the same three meanings with the
/// colour gone, because the sign is in the row's own text.
pub(crate) const fn token_of(ink: Ink) -> Token {
    match ink {
        Ink::Added => Token::Ok,
        Ink::Removed => Token::Bad,
        Ink::Context | Ink::Withheld => Token::Dim,
        Ink::Plain => Token::Fg,
    }
}

/// One row of the panel before it is painted: its words and the token they are
/// painted with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Painted {
    /// The row's text, exactly `width` cells across.
    pub(crate) text: String,
    /// The meaning the row is painted with.
    pub(crate) token: Token,
}

/// The rows the panel takes on a `screen_rows` screen, given at most `budget`
/// rows to spend.
pub(crate) fn rows(
    model: &Model,
    facts: &Facts,
    screen_rows: u16,
    width: u16,
    budget: usize,
) -> usize {
    panel(model, facts, screen_rows, width, budget).len()
}

/// The panel itself, as the lines the frame paints.
pub(crate) fn card(
    model: &Model,
    facts: &Facts,
    screen_rows: u16,
    width: u16,
    budget: usize,
) -> Vec<Line<'static>> {
    panel(model, facts, screen_rows, width, budget)
        .into_iter()
        .map(|row| Line::from(Span::styled(row.text, style(row.token, facts.scheme))))
        .collect()
}

/// The panel's rows: its opening rule with the title, where the call runs, the
/// call's own bytes in a block, the answers one per line, and its closing rule.
fn panel(
    model: &Model,
    facts: &Facts,
    screen_rows: u16,
    width: u16,
    budget: usize,
) -> Vec<Painted> {
    let Some(question) = model.card() else {
        return Vec::new();
    };
    let card = approval_waiting(
        question,
        facts.workspace.as_deref(),
        screen_rows,
        model.card_waiting(),
    );
    let charset = facts.charset;
    let width = width as usize;
    // The inside of the box: a rule either side and one cell of air beside it.
    let room = width.saturating_sub(4).max(1);
    // The seat that holds the keyboard is the one painted loudly: the panel's
    // rule is the accent while it answers, and dimmed while the composer does.
    let focused = model.panel_focused();
    let border = if focused { Token::Accent } else { Token::Dim };
    let mut out = vec![rule(card.title(), border, width, charset, true)];
    // What the budget leaves the body: the answers and both rules are not
    // negotiable, so everything else gives way to them.
    let mut spare = budget.saturating_sub(2 + card.choices().len());
    let place = card.place_rows(room);
    if !place.is_empty() && spare >= place.len() + 2 {
        for row in &place {
            out.push(inside(row, Token::Dim, width, charset));
        }
        out.push(inside("", Token::Fg, width, charset));
        spare -= place.len() + 1;
    }
    let shown = card.shown_rows(model.card_full(), room);
    let take = shown.len().min(spare.max(1));
    for row in &shown[..take] {
        out.push(inside(row.text(), token_of(row.ink), width, charset));
    }
    if spare > take {
        out.push(inside("", Token::Fg, width, charset));
    }
    for (index, choice) in card.choices().iter().enumerate() {
        let focused = model.card_choice() == Some(index);
        let marker = if focused {
            glyph(Glyph::Selected, charset)
        } else {
            " "
        };
        let token = if focused { Token::Accent } else { Token::Fg };
        out.push(inside(
            &format!("{marker} [{}] {}", choice.key, choice.label),
            token,
            width,
            charset,
        ));
    }
    out.push(rule(card.hint(focused), Token::Dim, width, charset, false));
    out
}

/// A rule across the content column, naming `label` where it opens (`top`) or
/// closes.
fn rule(label: &str, token: Token, width: usize, charset: Charset, top: bool) -> Painted {
    let (left, right) = match top {
        true => (Glyph::TopLeft, Glyph::TopRight),
        false => (Glyph::BottomLeft, Glyph::BottomRight),
    };
    let label = fit(label, width.saturating_sub(4));
    let fill = width.saturating_sub(label.chars().count() + 4);
    let line = glyph(Glyph::Rule, charset);
    Painted {
        text: format!(
            "{} {label} {}{}",
            glyph(left, charset),
            line.repeat(fill),
            glyph(right, charset)
        ),
        token,
    }
}

/// One row inside the box: the words, one cell in from each rule.
fn inside(text: &str, token: Token, width: usize, charset: Charset) -> Painted {
    let edge = glyph(Glyph::Edge, charset);
    let room = width.saturating_sub(4);
    Painted {
        text: format!("{edge} {} {edge}", pad(text, room)),
        token,
    }
}
