//! The saved-investigation picker overlay's rendering: a bordered modal with
//! a filter line, the matching rows (sanitized, truncated to the overlay
//! width), a no-match line, and a "more exist" note when the bounded load
//! was capped. The behaviour lives in `application/investigation_picker.rs`.

use super::super::theme::{accent, centered, foreground, on_accent, secondary};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};

/// The row a list entry renders as: sanitized (control characters become
/// spaces) and truncated to the overlay's inner width. Pure.
pub(super) fn display_row(label: &str, width: usize) -> String {
    label
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(width)
        .collect()
}

/// Draws the investigation picker overlay (filter-as-you-type over id + name).
pub(crate) fn draw_investigation_picker(
    frame: &mut Frame<'_>,
    app: &crate::interactive::tui::types::App,
    screen: Rect,
) {
    let Some(picker) = &app.overlays.investigations else {
        return;
    };
    let visible = app.investigations_visible(picker);
    let row_width = (screen.width.clamp(40, 90) as usize).saturating_sub(3);
    let mut lines: Vec<Line> = Vec::new();
    if !picker.query.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("filter: {}▏", picker.query),
            Style::default().fg(foreground()),
        )));
    }
    if visible.is_empty() {
        lines.push(Line::from(Span::styled(
            " no saved investigations match",
            Style::default().fg(secondary()),
        )));
    }
    for (i, entry) in visible.iter().take(12).enumerate() {
        let style = if i == picker.selected {
            Style::default()
                .bg(accent())
                .fg(on_accent())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!(" {}", display_row(&entry.label, row_width)),
            style,
        )));
    }
    if picker.capped {
        lines.push(Line::from(Span::styled(
            " more saved investigations — refine the filter",
            Style::default().fg(secondary()),
        )));
    }
    let rows = (lines.len() as u16).min(14);
    let height = (rows + 2).min(screen.height);
    let width = screen.width.clamp(40, 90);
    let area = centered(screen, width, height);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(accent()))
        .title(Span::styled(
            " saved investigations — type to filter · ↑/↓ · Enter show · r run · Esc ",
            Style::default().fg(accent()),
        ));
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(Text::from(lines)).block(block), area);
}

#[cfg(test)]
#[path = "investigation_picker_tests.rs"]
mod tests;
