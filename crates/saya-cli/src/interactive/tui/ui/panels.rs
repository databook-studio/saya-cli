//! Transcript and empty-state rendering.

use super::markdown::markdown_spans_fenced;
use super::splash::{
    NO_DATABASE_FOOTER, NO_DATABASE_HEADLINE, NO_DATABASE_STEPS, NO_WORKSPACE_LINES, splash_art,
};
use super::theme::{accent, kind_style, rail_style, secondary, warning};
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

    let mut lines: Vec<Line> = Vec::new();
    // ``` fence state persists across the consecutive lines of one assistant
    // block; any other role ends it.
    let mut fence = false;
    for (kind, text) in app.transcript.wide_view(width, height, &app.wide_table) {
        if text.is_empty() {
            lines.push(Line::from(""));
            continue;
        }
        // Shape differs per role so state survives without colour.
        let glyph = match kind {
            BlockKind::User => "❯ ",
            BlockKind::Assistant => "◆ ",
            BlockKind::Tool => "▸ ",
            BlockKind::Table => "▸ ",
            BlockKind::Error => "✗ ",
            BlockKind::System => "· ",
            BlockKind::Thinking => "≈ ",
        };
        let rail = Span::styled(glyph, rail_style(kind));
        let mut spans = vec![rail];
        if kind == BlockKind::Assistant {
            spans.extend(markdown_spans_fenced(&text, &mut fence));
        } else {
            fence = false;
            spans.push(Span::styled(text, kind_style(kind)));
        }
        lines.push(Line::from(spans));
    }
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
///
/// `workspace_bound` decides the workspace paragraph: `None` (no root
/// bound) draws the unbound line beside — never instead of — the
/// no-database guidance, so the two orthogonal absences both read.
pub(super) fn draw_empty_state(
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
