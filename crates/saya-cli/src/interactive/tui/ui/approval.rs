//! The docked approval panel: what will run, and the answers that decide it.

use super::theme::accent;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Paragraph, Wrap},
};

/// Computes the required vertical height (in rows) for the approval panel.
/// The answers line wraps like any other line, so a long offered token
/// claims its rows instead of clipping. The per-call fact bodies
/// (`approval_facts`) run longer than the old SQL-only detail, so the cap
/// follows them: an interpreter `run_program` prompt carries the no-euphemism
/// warning, and clipping a containment fact is worse than a taller panel.
pub(super) fn approval_height(detail: Option<&str>, grant: Option<&str>, width: u16) -> u16 {
    let inner = width.saturating_sub(2).max(1) as usize;
    let answers_rows = crate::grant_token::session_answers_line(grant)
        .chars()
        .count()
        .max(1)
        .div_ceil(inner)
        .max(1) as u16;
    match detail {
        Some(d) => {
            let wrapped_lines: usize = d
                .lines()
                .map(|line| line.chars().count().max(1).div_ceil(inner))
                .sum();
            (6 + wrapped_lines as u16 + answers_rows - 1).min(24)
        }
        None => (4 + answers_rows).min(16),
    }
}

/// Draws the tool-approval panel into the given area. The body is the shared
/// fact text — the same lines the terminal prompt renders
/// (`approval_facts::call_facts`) — drawn verbatim, so the modal cannot
/// state different facts for the same call; the answers line is the shared
/// three-answer text — with the offered token when one exists, two answers
/// and the reason when not.
pub(super) fn draw_approval(
    frame: &mut Frame<'_>,
    tool: &str,
    detail: Option<&str>,
    grant: Option<&str>,
    area: Rect,
) {
    let mut lines = Vec::new();
    match detail {
        Some(body) => {
            for line in body.lines() {
                lines.push(Line::from(Span::styled(
                    line.to_string(),
                    if line.starts_with("  ") {
                        Style::default().fg(accent())
                    } else {
                        Style::default().add_modifier(Modifier::BOLD)
                    },
                )));
            }
        }
        None => {
            lines.push(Line::from(format!("Run tool `{tool}`?")));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        crate::grant_token::session_answers_line(grant),
        Style::default().add_modifier(Modifier::BOLD),
    )));

    let block = ratatui::widgets::Block::default()
        .borders(ratatui::widgets::Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(super::theme::warning()))
        .title(Span::styled(
            " approval required ",
            Style::default()
                .fg(super::theme::warning())
                .add_modifier(Modifier::BOLD),
        ));

    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}
