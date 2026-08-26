//! Transcript and empty-state rendering.

use super::markdown::markdown_spans;
use super::theme::{accent, kind_style, rail_style, secondary};
use crate::interactive::tui::transcript::BlockKind;
use crate::interactive::tui::types::App;
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap},
};

/// Spinner frames shown while an agent request is streaming.
pub(super) const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Renders the visible, soft-wrapped transcript lines with a left role rail
/// and per-kind styling, plus a scrollbar when the content overflows.
pub(super) fn draw_transcript(frame: &mut Frame<'_>, app: &App, area: Rect) {
    // Reserve two columns on the left for the role rail; wrap text to the rest.
    let text_width = area.width.saturating_sub(2);
    // Store the WRAP width (not the pane width) so key-driven scrolling clamps consistently.
    app.viewport.set((text_width, area.height));
    let width = text_width as usize;
    let height = area.height as usize;

    let lines: Vec<Line> = app
        .transcript
        .view(width, height)
        .into_iter()
        .map(|(kind, text)| {
            if text.is_empty() {
                return Line::from("");
            }
            // Shape differs per role so state survives without colour.
            let glyph = match kind {
                BlockKind::User => "❯ ",
                BlockKind::Assistant => "◆ ",
                BlockKind::Tool => "▸ ",
                BlockKind::Error => "✗ ",
                BlockKind::System => "· ",
            };
            let rail = Span::styled(glyph, rail_style(kind));
            let mut spans = vec![rail];
            if kind == BlockKind::Assistant {
                spans.extend(markdown_spans(&text));
            } else {
                spans.push(Span::styled(text, kind_style(kind)));
            }
            Line::from(spans)
        })
        .collect();
    frame.render_widget(Paragraph::new(Text::from(lines)), area);

    let (total, first_visible) = app.transcript.scroll_metrics(width, height);
    if total > height {
        let mut state = ScrollbarState::new(total).position(first_visible);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .thumb_style(Style::default().fg(accent())),
            area,
            &mut state,
        );
    }
}

/// Draws a centered splash/empty state shown before the first conversation turn.
pub(super) fn draw_empty_state(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let mut content = Vec::with_capacity(11);

    content.push(Line::from(Span::styled(
        "◆ saya",
        Style::default().fg(accent()).add_modifier(Modifier::BOLD),
    )));
    content.push(Line::from(Span::styled(
        "Ask your databases in plain language.",
        Style::default().fg(secondary()),
    )));
    content.push(Line::from(""));

    if app.profiles.is_empty() {
        content.push(Line::from(Span::styled(
            "no database configured — type /connect",
            Style::default().fg(secondary()),
        )));
    } else {
        let mut spans = vec![Span::styled(
            "databases  ",
            Style::default().fg(secondary()),
        )];
        for (i, profile) in app.profiles.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled("  ·  ", Style::default().fg(secondary())));
            }
            spans.push(Span::styled(
                profile.as_str(),
                Style::default().fg(accent()),
            ));
        }
        content.push(Line::from(spans));
    }
    content.push(Line::from(""));

    content.push(Line::from(Span::styled(
        "try asking",
        Style::default().fg(secondary()),
    )));
    content.push(Line::from(Span::styled(
        "  which tables track billing?",
        Style::default()
            .fg(secondary())
            .add_modifier(Modifier::ITALIC),
    )));
    content.push(Line::from(Span::styled(
        "  top 5 customers by revenue",
        Style::default()
            .fg(secondary())
            .add_modifier(Modifier::ITALIC),
    )));
    content.push(Line::from(Span::styled(
        "  compare row counts across the connected databases",
        Style::default()
            .fg(secondary())
            .add_modifier(Modifier::ITALIC),
    )));
    content.push(Line::from(""));

    content.push(Line::from(Span::styled(
        "/ commands     @ tables     ? help     Ctrl+C quit",
        Style::default().fg(secondary()),
    )));

    let content_len = content.len();
    let pad = (area.height as usize).saturating_sub(content_len) / 2;

    let mut lines = Vec::with_capacity(pad + content_len);
    for _ in 0..pad {
        lines.push(Line::from(""));
    }
    lines.extend(content);

    frame.render_widget(
        Paragraph::new(Text::from(lines)).alignment(Alignment::Center),
        area,
    );
}

/// Computes the required vertical height (in rows) for the approval panel.
pub(super) fn approval_height(detail: Option<&str>, width: u16) -> u16 {
    let inner = width.saturating_sub(2).max(1) as usize;
    if let Some(d) = detail {
        let wrapped_lines: usize = d
            .lines()
            .map(|line| line.chars().count().max(1).div_ceil(inner))
            .sum();
        (6 + wrapped_lines as u16).min(16)
    } else {
        5
    }
}

/// Draws the tool-approval panel into the given area.
pub(super) fn draw_approval(frame: &mut Frame<'_>, tool: &str, detail: Option<&str>, area: Rect) {
    let mut lines = Vec::new();
    if let Some(sql) = detail {
        lines.push(Line::from(Span::styled(
            "Approve this read-only query:",
            Style::default().add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));
        for l in sql.lines() {
            lines.push(Line::from(Span::styled(
                l.to_string(),
                Style::default().fg(accent()),
            )));
        }
    } else {
        lines.push(Line::from(format!("Run tool `{tool}`?")));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "[y] allow    [n] deny",
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
