//! The startup workspace-trust modal: the TUI's rendering of the one
//! trust decision — trust this folder for the session, name a different
//! directory, or continue unbound — opened once after the splash paints.
//! The body is the shared prompt's own words (`TRUST_PROMPT_BODY`) and the
//! exits are the modal's answers line; the line prompt states the same
//! exits in prose (`TRUST_PROMPT_EXITS`). The two surfaces share every word
//! and only the key handling and the layout differ (modal keys here, line
//! reads there), so they cannot drift — and the modal never replays the
//! line prompt's single line, whose reflow could put the question and its
//! exits back on one row, indistinguishable on screen from a raw stdin
//! prompt in front of the alternate screen.

use super::super::theme::{foreground, secondary, warning};
use crate::interactive::session::permissions::trust::{TRUST_PROMPT_BODY, TRUST_PROMPT_NOTE};
use crate::interactive::tui::types::TrustPrompt;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
};

/// The modal's answers line: the same exits the line prompt states in
/// prose, laid out as a TUI choice row.
const ANSWERS: &str = "[t]rust once    [w]orkspace <dir> instead    [c]ontinue unbound";

/// How tall the trust modal may grow: the wrapped prompt body plus the
/// answers, note, draft-error, and draft rows, bounded so a narrow terminal
/// wraps instead of pushing the input off screen.
pub(in crate::interactive::tui) fn trust_modal_height(prompt: &TrustPrompt, width: u16) -> u16 {
    let inner = width.saturating_sub(2).max(1) as usize;
    let wrapped: usize = TRUST_PROMPT_BODY
        .lines()
        .map(|line| line.chars().count().max(1).div_ceil(inner))
        .sum();
    // Borders (2) + the body + the blank spacer + the answers line + the
    // session-only note.
    let rows = 5 + wrapped as u16 + u16::from(prompt.error.is_some());
    let draft_extra: u16 = prompt
        .draft
        .as_deref()
        .map_or(0, |draft| draft_rows(Some(draft), inner));
    rows.saturating_add(draft_extra).min(20)
}

/// Rows the typed `w <dir>` draft claims: the draft line itself, wrapped.
/// `None` (no `w` line open) claims no row.
fn draft_rows(draft: Option<&str>, inner: usize) -> u16 {
    let Some(draft) = draft else { return 0 };
    (format!("w {draft}").chars().count().max(1).div_ceil(inner) as u16).max(1)
}

/// Draws the trust modal: the shared prompt body, the modal's answers
/// line, and the `w <dir>` draft being typed with its inline refusal.
pub(in crate::interactive::tui) fn draw_trust_modal(
    frame: &mut Frame<'_>,
    prompt: &TrustPrompt,
    area: Rect,
) {
    let mut lines = Vec::new();
    for line in TRUST_PROMPT_BODY.lines() {
        lines.push(Line::from(Span::styled(
            line.to_string(),
            Style::default().fg(foreground()),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        ANSWERS,
        Style::default().add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        TRUST_PROMPT_NOTE,
        Style::default().fg(secondary()),
    )));
    if prompt.draft.is_some() || prompt.error.is_some() {
        lines.push(Line::from(Span::styled(
            format!("w {}▏", prompt.draft.as_deref().unwrap_or("")),
            Style::default().fg(secondary()),
        )));
    }
    if let Some(error) = &prompt.error {
        lines.push(Line::from(Span::styled(
            error.clone(),
            Style::default().fg(warning()),
        )));
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(warning()))
        .title(Span::styled(
            " trust this folder? ",
            Style::default().fg(warning()).add_modifier(Modifier::BOLD),
        ));
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

#[cfg(test)]
#[path = "trust_modal_tests.rs"]
mod tests;
