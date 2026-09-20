//! The splash / empty state shown before the first conversation turn.

use super::super::theme::{accent, secondary, warning};
use super::splash::{
    NO_DATABASE_FOOTER, NO_DATABASE_HEADLINE, NO_DATABASE_STEPS, NO_WORKSPACE_LINES, splash_art,
};
use crate::interactive::tui::types::App;
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::Paragraph,
};

/// Draws a centered splash/empty state shown before the first conversation turn.
///
/// `workspace_bound` decides the workspace paragraph: `None` (no root
/// bound) draws the unbound line beside — never instead of — the
/// no-database guidance, so the two orthogonal absences both read.
pub(in crate::interactive::tui) fn draw_empty_state(
    frame: &mut Frame<'_>,
    app: &App,
    area: Rect,
    workspace_bound: Option<bool>,
) {
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
        // The text before this sent new users to /connect, which can only select
        // already-configured profiles — a dead end. The copy lives in `splash`
        // so it can be asserted on; see the tests there for what it must hold.
        content.push(Line::from(Span::styled(
            NO_DATABASE_HEADLINE,
            Style::default().fg(warning()).add_modifier(Modifier::BOLD),
        )));
        for step in NO_DATABASE_STEPS {
            content.push(Line::from(Span::styled(
                step,
                Style::default().fg(secondary()),
            )));
        }
        content.push(Line::from(""));
        content.push(Line::from(Span::styled(
            NO_DATABASE_FOOTER,
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
    // The unbound-workspace paragraph, beside — never instead of — the
    // no-database guidance above: the two absences are orthogonal and both
    // must read. `Some(true)` (a root bound) draws nothing, keeping a bound
    // session's splash byte-identical.
    if workspace_bound == Some(false) {
        content.push(Line::from(""));
        for line in NO_WORKSPACE_LINES {
            content.push(Line::from(Span::styled(
                line,
                Style::default().fg(warning()).add_modifier(Modifier::BOLD),
            )));
        }
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

    if let Some(mut with_art) = splash_art(area.height as usize, content.len()) {
        with_art.push(Line::from(""));
        with_art.extend(content);
        content = with_art;
    }

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
