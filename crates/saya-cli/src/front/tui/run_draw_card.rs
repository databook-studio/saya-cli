//! The docked card's rows: the call the operator has to answer, laid out as the
//! lines the frame paints.
//!
//! The words are [`approval`]'s — the caller's title, the call's facts verbatim,
//! why it is asked and the keys that answer it. This module only says how many
//! rows that is and turns them into lines, so the grid can reserve them before
//! anything is painted and the card and the grid can never disagree about the
//! card's height.

use ratatui::text::{Line, Span};

use super::model::Model;
use super::run_draw::Facts;
use super::theme::{Token, style};
use super::view::{Card, approval};

/// The rows the card needs: its title, the body rows it shows, and the line
/// that says why it is asked and which keys answer it.
pub(crate) fn rows(model: &Model, screen_rows: u16) -> usize {
    model.card().map_or(0, |question| {
        let card = approval(question, model.mode(), screen_rows);
        1 + card.shown(model.card_full()).len() + 1
    })
}

/// The card itself: the call's own title, its body verbatim, its keys.
pub(crate) fn card(model: &Model, facts: &Facts, screen_rows: u16) -> Vec<Line<'static>> {
    let Some(question) = model.card() else {
        return Vec::new();
    };
    let card = approval(question, model.mode(), screen_rows);
    let mut lines = vec![Line::from(Span::styled(
        card.title().to_owned(),
        style(Token::Accent, facts.scheme),
    ))];
    for row in card.shown(model.card_full()) {
        lines.push(Line::from(Span::styled(
            row.clone(),
            style(Token::Fg, facts.scheme),
        )));
    }
    lines.push(Line::from(Span::styled(
        tail(&card),
        style(Token::Dim, facts.scheme),
    )));
    lines
}

/// The card's last line: why it is asked, then the keys that answer it. A card
/// that offers no key (a clarification) ends with the reason alone.
fn tail(card: &Card) -> String {
    let mut said = card.because().map(str::to_owned).unwrap_or_default();
    if !card.keys().is_empty() {
        if !said.is_empty() {
            said.push_str(" · ");
        }
        said.push_str(card.keys());
    }
    said
}
