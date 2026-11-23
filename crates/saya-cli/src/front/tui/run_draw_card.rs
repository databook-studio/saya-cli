//! The docked card's rows: the call the operator has to answer, laid out as the
//! lines the frame paints.
//!
//! The words are [`super::view::cards`]'s — the title, the call's own bytes,
//! where it runs and the answers it offers. This module says how many rows that
//! is and turns them into lines, so the grid can reserve them before anything
//! is painted and the card and the grid can never disagree about the card's
//! height.

use ratatui::text::{Line, Span};

use super::model::Model;
use super::run_draw::Facts;
use super::theme::{Token, style};
use super::view::approval_waiting;

/// One row of the panel, before it is painted: its words and the token they are
/// painted with.
pub(crate) type Painted = (String, Token);

/// The rows the card needs, given at most `budget` rows to spend.
pub(crate) fn rows(model: &Model, facts: &Facts, screen_rows: u16, width: u16, budget: usize) -> usize {
    lines(model, facts, screen_rows, width, budget).len()
}

/// The card itself, as the lines the frame paints.
pub(crate) fn card(
    model: &Model,
    facts: &Facts,
    screen_rows: u16,
    width: u16,
    budget: usize,
) -> Vec<Line<'static>> {
    lines(model, facts, screen_rows, width, budget)
        .into_iter()
        .map(|(text, token)| Line::from(Span::styled(text, style(token, facts.scheme))))
        .collect()
}

/// The card's rows: its title, the call's own bytes, where it runs and the
/// answers it offers.
fn lines(model: &Model, facts: &Facts, screen_rows: u16, width: u16, budget: usize) -> Vec<Painted> {
    let Some(question) = model.card() else {
        return Vec::new();
    };
    let card = approval_waiting(
        question,
        facts.workspace.as_deref(),
        screen_rows,
        model.card_waiting(),
    );
    let mut out = vec![(card.title().to_owned(), Token::Accent)];
    if let Some(place) = card.place() {
        out.push((place.to_owned(), Token::Dim));
    }
    for row in card.shown_rows(model.card_full(), width as usize) {
        out.push((row, Token::Fg));
    }
    for choice in card.choices() {
        out.push((format!("[{}] {}", choice.key, choice.label), Token::Fg));
    }
    out.truncate(budget);
    out
}
