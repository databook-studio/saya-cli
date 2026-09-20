//! The startup workspace-trust modal: the TUI's rendering of the one
//! trust decision — trust this folder for the session, name a different
//! directory, or continue unbound — opened once after the splash paints.
//! The body is the shared `TRUST_PROMPT` text the plain REPL prints
//! verbatim; only the key handling differs (modal keys here, line reads
//! there), so the two surfaces cannot drift.

use super::super::theme::{foreground, secondary, warning};
use crate::interactive::session_trust::TRUST_PROMPT;
use crate::interactive::tui::types::TrustPrompt;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
};

/// How tall the trust modal may grow: the wrapped prompt body plus the
/// answers, draft-error, and draft rows, bounded so a narrow terminal
/// wraps instead of pushing the input off screen.
pub(in crate::interactive::tui) fn trust_modal_height(prompt: &TrustPrompt, width: u16) -> u16 {
    let inner = width.saturating_sub(2).max(1) as usize;
    let wrapped: usize = TRUST_PROMPT
        .lines()
        .map(|line| line.chars().count().max(1).div_ceil(inner))
        .sum();
    let rows = 4 + wrapped as u16 + u16::from(prompt.error.is_some());
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
    for line in TRUST_PROMPT.lines() {
        lines.push(Line::from(Span::styled(
            line.to_string(),
            Style::default().fg(foreground()),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "[t]rust once    [w]orkspace <dir> instead    [c]ontinue unbound",
        Style::default().add_modifier(Modifier::BOLD),
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
