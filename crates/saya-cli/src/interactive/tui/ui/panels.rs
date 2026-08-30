//! Transcript and empty-state rendering.

use super::markdown::markdown_spans_fenced;
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

/// The splash mascot, in two sizes. Every row is padded to the same width so
/// `Alignment::Center` shifts them all by the same amount — a ragged row would
/// centre on its own width and skew the art. The `▌` is the cursor mouth and is
/// styled separately, so it reads as a cursor rather than as more of the body.
const SPLASH_ART: [&str; 8] = [
    "      \u{2588}      ",
    "    \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}    ",
    "  \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}  ",
    "\u{2588}\u{2588}\u{2588}  \u{2588}\u{2588}\u{2588}  \u{2588}\u{2588}\u{2588}",
    "  \u{2588}\u{2588}\u{2588} \u{258c} \u{2588}\u{2588}\u{2588}  ",
    "    \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}    ",
    "      \u{2588}      ",
    "      \u{2591}\u{2591}\u{2591}\u{2591}\u{2591}\u{2591} ",
];

/// The compact mascot, used when the full one would push the splash off-screen.
const SPLASH_ART_COMPACT: [&str; 5] = [
    "    \u{2588}    ",
    "  \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}  ",
    "\u{2588}\u{2588}  \u{2588}  \u{2588}\u{2588}",
    "  \u{2588} \u{258c} \u{2588}  ",
    "    \u{2588}    ",
];

/// Builds the mascot rows: the body carries the accent, the cast-shadow row
/// recedes into secondary, and the cursor mouth takes the foreground so it
/// reads as a cursor.
fn splash_art_lines(rows: &[&'static str]) -> Vec<Line<'static>> {
    rows.iter()
        .map(|row| {
            // The cast-shadow row is the only one built from the shade glyph.
            if row.contains('\u{2591}') {
                return Line::from(Span::styled(*row, Style::default().fg(secondary())));
            }
            let Some(mouth) = row.find('\u{258c}') else {
                return Line::from(Span::styled(*row, Style::default().fg(accent())));
            };
            let (head, rest) = row.split_at(mouth);
            let (cursor, tail) = rest.split_at('\u{258c}'.len_utf8());
            Line::from(vec![
                Span::styled(head, Style::default().fg(accent())),
                Span::styled(cursor, Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(tail, Style::default().fg(accent())),
            ])
        })
        .collect()
}

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
    for (kind, text) in app.transcript.view(width, height) {
        if text.is_empty() {
            lines.push(Line::from(""));
            continue;
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
        // The old text sent new users to /connect, which can only select
        // already-configured profiles — a dead end. Point at the real path.
        content.push(Line::from(Span::styled(
            "No database is configured yet.",
            Style::default().fg(warning()).add_modifier(Modifier::BOLD),
        )));
        content.push(Line::from(Span::styled(
            "Run `saya config init`, add a profile to .saya/connections.toml,",
            Style::default().fg(secondary()),
        )));
        content.push(Line::from(Span::styled(
            "then run `saya connection test <name>` and restart.",
            Style::default().fg(secondary()),
        )));
        content.push(Line::from(""));
        content.push(Line::from(Span::styled(
            "Check problems any time with `saya config doctor`.",
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

    // Fit the largest mascot that still leaves the whole splash on screen. A
    // short terminal drops it rather than pushing the tagline and hints off the
    // top — the art is decoration, the text is the thing that has to be read.
    let height = area.height as usize;
    let art = if height > content.len() + SPLASH_ART.len() {
        Some(&SPLASH_ART[..])
    } else if height > content.len() + SPLASH_ART_COMPACT.len() {
        Some(&SPLASH_ART_COMPACT[..])
    } else {
        None
    };
    if let Some(rows) = art {
        let mut with_art = splash_art_lines(rows);
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
