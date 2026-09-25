//! The slash-command popup floating above the input box.

use super::super::theme::{accent, foreground, on_accent, secondary};
use crate::interactive::tui::complete::Candidate;
use crate::interactive::tui::types::Menu;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};

/// Largest number of rows shown in the slash-command popup.
const MAX_MENU_ROWS: usize = 8;

/// Renders the slash-command popup floating just above the input box.
pub(in crate::interactive::tui) fn draw_menu(frame: &mut Frame<'_>, menu: &Menu, input_area: Rect) {
    let rows = menu.candidates.len().min(MAX_MENU_ROWS);
    let height = rows as u16 + 2;
    let width = input_area.width.clamp(24, 68);
    let area = Rect {
        x: input_area.x,
        y: input_area.y.saturating_sub(height),
        width,
        height,
    };
    let inner_width = width.saturating_sub(2) as usize;
    let offset = menu.selected.saturating_sub(MAX_MENU_ROWS - 1);
    let lines: Vec<Line> = menu
        .candidates
        .iter()
        .enumerate()
        .skip(offset)
        .take(MAX_MENU_ROWS)
        .map(|(i, candidate)| menu_row(candidate, i == menu.selected, inner_width))
        .collect();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(accent()))
        .title(Span::styled(" commands ", Style::default().fg(accent())));
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(Text::from(lines)).block(block), area);
}

/// Builds one popup row: the value plus a dimmed description, with the selected
/// row filled edge-to-edge in the accent color.
fn menu_row(candidate: &Candidate, selected: bool, width: usize) -> Line<'static> {
    if selected {
        let label = match &candidate.description {
            Some(desc) => format!("{}  {desc}", candidate.value),
            None => candidate.value.clone(),
        };
        let padded = format!("{label:<width$}");
        return Line::from(Span::styled(
            padded,
            Style::default()
                .bg(accent())
                .fg(on_accent())
                .add_modifier(Modifier::BOLD),
        ));
    }
    let mut spans = vec![Span::styled(
        candidate.value.clone(),
        Style::default().fg(foreground()),
    )];
    if let Some(desc) = &candidate.description {
        spans.push(Span::styled(
            format!("  {desc}"),
            Style::default().fg(secondary()),
        ));
    }
    Line::from(spans)
}
