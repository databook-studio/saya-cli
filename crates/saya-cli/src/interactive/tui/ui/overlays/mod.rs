//! Modal overlays: approval, plan approval, trust modal, run panel,
//! slash-command popup, session picker, and help.

mod approval;
mod menu;
mod plan_approval;
mod run_panel;
mod trust_modal;

pub(super) use approval::{approval_height, draw_approval};
pub(super) use menu::draw_menu;
pub(super) use plan_approval::{draw_plan_approval, plan_approval_height};
pub(super) use run_panel::{draw_run_panel, run_panel_height};
pub(super) use trust_modal::{draw_trust_modal, trust_modal_height};

use super::theme::{accent, centered, foreground, on_accent, secondary};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};

/// Draws the session picker overlay (filter-as-you-type over id + label).
pub(super) fn draw_picker(
    frame: &mut Frame<'_>,
    app: &crate::interactive::tui::types::App,
    screen: Rect,
) {
    let Some(picker) = &app.overlays.picker else {
        return;
    };
    let visible = app.picker_visible(picker);
    if picker.selected >= visible.len().max(1) && !visible.is_empty() {
        // Selection clamped by move(); nothing to do here.
    }
    let mut lines: Vec<Line> = Vec::new();
    if !picker.query.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("filter: {}▏", picker.query),
            Style::default().fg(foreground()),
        )));
    }
    if visible.is_empty() {
        lines.push(Line::from(Span::styled(
            " no sessions match",
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
        lines.push(Line::from(Span::styled(format!(" {}", entry.label), style)));
    }
    if picker.has_more {
        lines.push(Line::from(Span::styled(
            " more sessions available — refine the filter",
            Style::default().fg(secondary()),
        )));
    }
    let rows = (lines.len() as u16).min(13);
    let height = (rows + 2).min(screen.height);
    let width = screen.width.clamp(40, 90);
    let area = centered(screen, width, height);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(accent()))
        .title(Span::styled(
            " resume session — type to filter · ↑/↓ · Enter · Esc ",
            Style::default().fg(accent()),
        ));
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(Text::from(lines)).block(block), area);
}

/// Draws the keybinding help overlay.
pub(super) fn draw_help(frame: &mut Frame<'_>, screen: Rect) {
    let lines = [
        "Enter        submit  ·  Alt+Enter  newline  ·  Enter on empty line expands/collapses latest tool group or chapter",
        "/            command popup  ·  @  table references",
        "Tab / Enter  accept popup suggestion  ·  Esc  dismiss",
        "↑ / ↓        history (input)  ·  overlay navigation",
        "Ctrl+R       search input history  ·  Ctrl+F  find in transcript",
        "PageUp/Dn    scroll transcript",
        "Alt+↑/↓      step to previous/next result  ·  unfolds its chapter",
        "Ctrl+,/.     scroll a wide result table ←/→  ·  Ctrl+P  pin first col",
        "Ctrl+A/E     start/end of line  ·  Ctrl+W/U  delete word/line",
        "Ctrl+C       cancel request / clear · twice to exit",
        "Ctrl+G       drop the queued prompt (while a request runs)",
        "Shift+End    return to the live edge (new lines below)",
        "Esc          cancel a running request",
        "Ctrl+O       selection mode (drag-select + copy)",
        "Ctrl+Y       copy last answer (last assistant block, not a table)",
        "Ctrl+B       copy transcript (full, minus reasoning)",
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

/// Draws the Ctrl+R / Ctrl+F search overlay.
pub(super) fn draw_search(
    frame: &mut Frame<'_>,
    app: &crate::interactive::tui::types::App,
    screen: Rect,
) {
    use crate::interactive::tui::types::SearchKind;
    let Some(search) = &app.overlays.search else {
        return;
    };
    let title = match search.kind {
        SearchKind::History => "search history — ↑/↓ select · Enter insert · Esc cancel",
        SearchKind::Transcript => "find in transcript — Enter next match · Esc cancel",
    };
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        format!("> {}▏", search.query),
        Style::default().fg(foreground()),
    )));
    match search.kind {
        SearchKind::History => {
            let matches = app.search_matches(search);
            if matches.is_empty() {
                lines.push(Line::from(Span::styled(
                    " no matches",
                    Style::default().fg(secondary()),
                )));
            }
            for (i, entry) in matches.iter().take(8).enumerate() {
                let style = if i == search.selected {
                    Style::default().bg(accent()).fg(on_accent())
                } else {
                    Style::default().fg(foreground())
                };
                let one_line: String = entry
                    .chars()
                    .map(|c| if c == '\n' { ' ' } else { c })
                    .collect();
                lines.push(Line::from(Span::styled(format!(" {one_line}"), style)));
            }
        }
        SearchKind::Transcript => {
            if !search.query.is_empty() {
                let count = app
                    .transcript
                    .count_matches(&search.query, screen.width.saturating_sub(2) as usize);
                lines.push(Line::from(Span::styled(
                    format!(" {count} matching line(s) — Enter to jump"),
                    Style::default().fg(secondary()),
                )));
            }
        }
    }
    let height = (lines.len() as u16 + 2).min(screen.height);
    let area = centered(screen, screen.width.clamp(40, 90), height);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(accent()))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(accent()),
        ));
    frame.render_widget(Clear, area);
    frame.render_widget(
        ratatui::widgets::Paragraph::new(Text::from(lines)).block(block),
        area,
    );
}
