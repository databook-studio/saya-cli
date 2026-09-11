//! The plan-approval modal: the M1-10 gate, shown once by the run panel's
//! worker and answered here. The view text is the drive's own rendering —
//! the same body the terminal driver prints for `saya run` — so the modal
//! shapes nothing of its own.

use super::theme::{accent, warning};
use crate::interactive::tui::run_panel::RunPanel;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Paragraph, Wrap},
};

/// The modal's height — the tool-approval panel's arithmetic: the wrapped
/// view text plus the prompt rows, bounded so a long plan cannot push the
/// input off screen.
pub(super) fn plan_approval_height(view_text: &str, width: u16) -> u16 {
    let inner = width.saturating_sub(2).max(1) as usize;
    let wrapped: usize = view_text
        .lines()
        .map(|line| line.chars().count().max(1).div_ceil(inner))
        .sum();
    (6 + wrapped as u16).min(18)
}

/// Draws the plan-approval modal: the same view text the terminal driver
/// prints for `saya run`, answered here with y/n — an explicit yes and
/// nothing else, exactly the tool-approval modal's rule.
pub(super) fn draw_plan_approval(frame: &mut Frame<'_>, panel: &RunPanel, area: Rect) {
    let Some(request) = panel.plan_approval.as_ref() else {
        return;
    };
    let mut lines = vec![
        Line::from(Span::styled(
            "Approve this plan, its scopes, and its budgets:",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];
    for l in request.view_text.lines() {
        lines.push(Line::from(Span::styled(
            l.to_string(),
            Style::default().fg(accent()),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "[y] approve    [n] refuse",
        Style::default().add_modifier(Modifier::BOLD),
    )));
    let block = Block::default()
        .borders(ratatui::widgets::Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(warning()))
        .title(Span::styled(
            " plan approval ",
            Style::default().fg(warning()).add_modifier(Modifier::BOLD),
        ));
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}
