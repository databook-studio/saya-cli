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
use super::view::approval;

/// The rows the card needs: its title, the body rows it shows, the reason it is
/// asked and the line that says which keys answer it.
pub(crate) fn rows(model: &Model, screen_rows: u16, host_unsandboxed: bool) -> usize {
    model.card().map_or(0, |question| {
        let card = approval(question, model.mode(), screen_rows);
        1 + card.shown(model.card_full()).len()
            + usize::from(card.because().is_some())
            + usize::from(host_unsandboxed)
            + usize::from(!card.keys().is_empty())
    })
}

/// The card itself: the call's own title, its body verbatim, why it is asked,
/// and the keys that answer it, fitted to `width` cells.
pub(crate) fn card(
    model: &Model,
    facts: &Facts,
    screen_rows: u16,
    width: u16,
) -> Vec<Line<'static>> {
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
    // Why it is asked and how it is answered are two rows: on one row the
    // reason is long enough to clip the keys off a narrow terminal.
    if let Some(because) = card.because() {
        lines.push(Line::from(Span::styled(
            because.to_owned(),
            style(Token::Dim, facts.scheme),
        )));
    }
    // The launch's lane is the card's own fact, on its own row: the operator
    // consenting to a host command is exactly who needs to read it.
    if facts.host_unsandboxed {
        lines.push(Line::from(Span::styled(
            "host commands run unsandboxed".to_owned(),
            style(Token::Warn, facts.scheme),
        )));
    }
    if !card.keys().is_empty() {
        lines.push(Line::from(Span::styled(
            card.keys_for(width as usize).to_owned(),
            style(Token::Dim, facts.scheme),
        )));
    }
    lines
}
