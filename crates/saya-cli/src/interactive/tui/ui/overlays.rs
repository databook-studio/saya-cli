//! Modal overlays: slash-command popup, session picker, and help.

use super::theme::{accent, centered, secondary};
use crate::interactive::tui::complete::Candidate;
use crate::interactive::tui::types::{Menu, Picker};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};

/// Largest number of rows shown in the slash-command popup.
const MAX_MENU_ROWS: usize = 8;

/// Draws the session picker overlay.
pub(super) fn draw_picker(frame: &mut Frame<'_>, picker: &Picker, screen: Rect) {
    let rows = (picker.entries.len() as u16).min(12);
    let height = (rows + 2).min(screen.height);
    let width = screen.width.clamp(40, 90);
    let area = centered(screen, width, height);
    let lines: Vec<Line> = picker
        .entries
        .iter()
        .take(12)
        .enumerate()
        .map(|(i, entry)| {
            let style = if i == picker.selected {
                Style::default()
                    .bg(accent())
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            Line::from(Span::styled(format!(" {}", entry.label), style))
        })
        .collect();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(accent()))
        .title(Span::styled(
            " resume session — ↑/↓ select · Enter resume · Esc cancel ",
            Style::default().fg(accent()),
        ));
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(Text::from(lines)).block(block), area);
}

/// Draws the keybinding help overlay.
pub(super) fn draw_help(frame: &mut Frame<'_>, screen: Rect) {
    let lines = [
        "Enter        submit  ·  Alt+Enter  newline",
        "/            command popup  ·  @  table references",
        "Tab / Enter  accept popup suggestion  ·  Esc  dismiss",
        "↑ / ↓        history (input)  ·  popup navigation",
        "PageUp/Dn    scroll transcript",
        "Ctrl+A/E     start/end of line  ·  Ctrl+W/U  delete word/line",
        "Ctrl+C       cancel request / clear · twice to exit",
        "Esc          cancel a running request",
        "Ctrl+O       selection mode (drag-select + copy)",
        "Ctrl+Y       copy last answer  ·  Ctrl+B  copy transcript",
        "? or F1      toggle this help",
    ];
    let width = screen.width.clamp(40, 72);
    let height = (lines.len() as u16 + 2).min(screen.height);
    let area = centered(screen, width, height);
    let body: Vec<Line> = lines.iter().map(|l| Line::from(*l)).collect();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(accent()))
        .title(Span::styled(
            " keybindings — any key to close ",
            Style::default().fg(accent()).add_modifier(Modifier::BOLD),
        ));
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(Text::from(body)).block(block), area);
}

/// Renders the slash-command popup floating just above the input box.
pub(super) fn draw_menu(frame: &mut Frame<'_>, menu: &Menu, input_area: Rect) {
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
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        ));
    }
    let mut spans = vec![Span::styled(
        candidate.value.clone(),
        Style::default().fg(Color::White),
    )];
    if let Some(desc) = &candidate.description {
        spans.push(Span::styled(
            format!("  {desc}"),
            Style::default().fg(secondary()),
        ));
    }
    Line::from(spans)
}
