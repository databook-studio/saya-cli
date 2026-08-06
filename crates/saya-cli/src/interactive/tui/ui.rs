//! Rendering for the TUI: transcript, status bar, input box, and the slash
//! popup. Kept separate from the event loop so styling can evolve on its own.

use super::transcript::BlockKind;
use super::{App, Menu};
use crate::interactive::session_prompt::StatusView;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, BorderType, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation,
        ScrollbarState, Wrap,
    },
};

/// Largest number of rows shown in the slash-command popup.
const MAX_MENU_ROWS: usize = 8;

/// saya's signature accent (iris violet): brand, assistant, focus, borders.
const ACCENT: Color = Color::Rgb(157, 139, 245);
/// User turns — a cool secondary so the accent stays saya's.
const USER_COLOR: Color = Color::Rgb(106, 155, 204);
/// Secondary/de-emphasised text: tool + system lines, hints, provider/model.
const SECONDARY: Color = Color::Rgb(168, 162, 154);
/// Status: success / safe (read-only approval, privacy on, passing checks).
const SUCCESS: Color = Color::Rgb(127, 174, 107);
/// Status: caution (ask approval, approval-panel border).
const WARNING: Color = Color::Rgb(224, 164, 88);
/// Status: error / danger (failures, never approval).
const DANGER: Color = Color::Rgb(229, 105, 95);
/// Status-bar / badge background (faint iris-tinted dark).
const STATUS_BG: Color = Color::Rgb(30, 28, 36);
/// Inline `code` in assistant answers.
const CODE_COLOR: Color = Color::Rgb(127, 181, 214);

/// Maps a transcript block kind to its display style.
fn kind_style(kind: BlockKind) -> Style {
    match kind {
        BlockKind::User => Style::default().fg(USER_COLOR).add_modifier(Modifier::BOLD),
        BlockKind::Assistant => Style::default().fg(Color::White),
        BlockKind::System => Style::default()
            .fg(SECONDARY)
            .add_modifier(Modifier::ITALIC),
        BlockKind::Error => Style::default().fg(DANGER).add_modifier(Modifier::BOLD),
        BlockKind::Tool => Style::default().fg(SECONDARY),
    }
}

/// Returns the rail color style for a given transcript block kind.
fn rail_style(kind: BlockKind) -> Style {
    match kind {
        BlockKind::User => Style::default().fg(USER_COLOR).add_modifier(Modifier::BOLD),
        BlockKind::Assistant => Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        BlockKind::Tool | BlockKind::System => Style::default().fg(SECONDARY),
        BlockKind::Error => Style::default().fg(DANGER).add_modifier(Modifier::BOLD),
    }
}

/// Draws one frame: transcript (fills), status bar, approval panel (when pending), input box, popup overlay.
pub(super) fn draw(frame: &mut Frame<'_>, app: &App, status: &StatusView) {
    let input_height = (app.input_rows() as u16) + 2;
    let approval_h = app
        .pending_approval
        .as_ref()
        .map(|p| approval_height(p.detail.as_deref(), frame.area().width))
        .unwrap_or(0);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),               // transcript
            Constraint::Length(1),            // status bar
            Constraint::Length(approval_h),   // approval panel (0 when none)
            Constraint::Length(input_height), // input box
        ])
        .split(frame.area());

    let has_turns = app
        .transcript
        .blocks()
        .iter()
        .any(|b| matches!(b.kind, BlockKind::User | BlockKind::Assistant));
    if has_turns {
        draw_transcript(frame, app, chunks[0]);
    } else {
        draw_empty_state(frame, app, chunks[0]);
    }
    draw_status(frame, app, status, chunks[1]);
    if let Some(pending) = &app.pending_approval {
        draw_approval(frame, &pending.tool, pending.detail.as_deref(), chunks[2]);
    }
    draw_input(frame, app, chunks[3]);
    if let Some(menu) = &app.menu {
        draw_menu(frame, menu, chunks[3]);
    }
    if let Some(picker) = &app.picker {
        draw_picker(frame, picker, frame.area());
    }
    if app.show_help {
        draw_help(frame, frame.area());
    }
}

/// Draws the session picker overlay.
fn draw_picker(frame: &mut Frame<'_>, picker: &super::Picker, screen: Rect) {
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
                    .bg(ACCENT)
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
        .border_style(Style::default().fg(ACCENT))
        .title(Span::styled(
            " resume session — ↑/↓ select · Enter resume · Esc cancel ",
            Style::default().fg(ACCENT),
        ));
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(Text::from(lines)).block(block), area);
}

/// Draws the keybinding help overlay.
fn draw_help(frame: &mut Frame<'_>, screen: Rect) {
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
        .border_style(Style::default().fg(ACCENT))
        .title(Span::styled(
            " keybindings — any key to close ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ));
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(Text::from(body)).block(block), area);
}

/// Centers a `width`×`height` rect within `screen`.
fn centered(screen: Rect, width: u16, height: u16) -> Rect {
    Rect {
        x: screen.x + (screen.width.saturating_sub(width)) / 2,
        y: screen.y + (screen.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}

/// Computes the required vertical height (in rows) for the approval panel.
fn approval_height(detail: Option<&str>, width: u16) -> u16 {
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
fn draw_approval(frame: &mut Frame<'_>, tool: &str, detail: Option<&str>, area: Rect) {
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
                Style::default().fg(ACCENT),
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

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(WARNING))
        .title(Span::styled(
            " approval required ",
            Style::default().fg(WARNING).add_modifier(Modifier::BOLD),
        ));

    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

/// Spinner frames shown while an agent request is streaming.
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Renders the visible, soft-wrapped transcript lines with a left role rail
/// and per-kind styling, plus a scrollbar when the content overflows.
fn draw_transcript(frame: &mut Frame<'_>, app: &App, area: Rect) {
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
            let rail = Span::styled("▎ ", rail_style(kind));
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
                .thumb_style(Style::default().fg(ACCENT)),
            area,
            &mut state,
        );
    }
}

/// Draws a centered splash/empty state shown before the first conversation turn.
fn draw_empty_state(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let mut content = Vec::with_capacity(11);

    content.push(Line::from(Span::styled(
        "◆ saya",
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    )));
    content.push(Line::from(Span::styled(
        "Ask your databases in plain language.",
        Style::default().fg(SECONDARY),
    )));
    content.push(Line::from(""));

    if app.profiles.is_empty() {
        content.push(Line::from(Span::styled(
            "no database configured — type /connect",
            Style::default().fg(SECONDARY),
        )));
    } else {
        let mut spans = vec![Span::styled("databases  ", Style::default().fg(SECONDARY))];
        for (i, profile) in app.profiles.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled("  ·  ", Style::default().fg(SECONDARY)));
            }
            spans.push(Span::styled(profile.as_str(), Style::default().fg(ACCENT)));
        }
        content.push(Line::from(spans));
    }
    content.push(Line::from(""));

    content.push(Line::from(Span::styled(
        "try asking",
        Style::default().fg(SECONDARY),
    )));
    content.push(Line::from(Span::styled(
        "  which tables track billing?",
        Style::default()
            .fg(SECONDARY)
            .add_modifier(Modifier::ITALIC),
    )));
    content.push(Line::from(Span::styled(
        "  top 5 customers by revenue",
        Style::default()
            .fg(SECONDARY)
            .add_modifier(Modifier::ITALIC),
    )));
    content.push(Line::from(Span::styled(
        "  compare row counts across the connected databases",
        Style::default()
            .fg(SECONDARY)
            .add_modifier(Modifier::ITALIC),
    )));
    content.push(Line::from(""));

    content.push(Line::from(Span::styled(
        "/ commands     @ tables     ? help     Ctrl+C quit",
        Style::default().fg(SECONDARY),
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

/// Builds the coloured status-bar segments (profile, provider/model, approval, privacy),
/// each on the bar background so they blend into the strip.
fn status_spans(view: &StatusView, bg: Color) -> Vec<Span<'static>> {
    let base = Style::default().bg(bg);
    let approval_color = match view.approval_mode.as_str() {
        "read-only" => SUCCESS,
        "ask" => WARNING,
        "never" => DANGER,
        _ => SECONDARY,
    };
    let mut label = view.profile.clone();
    for inc in &view.included {
        label.push_str(&format!(" +{inc}"));
    }
    vec![
        Span::styled(
            format!(" [{label}] "),
            base.fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{}/{} ", view.provider, view.model),
            base.fg(SECONDARY),
        ),
        Span::styled(
            format!("approval:{} ", view.approval_mode),
            base.fg(approval_color),
        ),
        Span::styled(
            format!("privacy:{}", if view.privacy_on { "on" } else { "off" }),
            base.fg(if view.privacy_on { SUCCESS } else { SECONDARY }),
        ),
    ]
}

/// Renders the status bar as a filled accent-tinted strip, with a spinner and
/// hint while an agent request is streaming.
fn draw_status(frame: &mut Frame<'_>, app: &App, status: &StatusView, area: Rect) {
    let bar = Style::default().bg(STATUS_BG).fg(SECONDARY);
    let bg = STATUS_BG;
    let line = if app.is_busy() {
        let frame_char = SPINNER[app.spinner % SPINNER.len()];
        let elapsed = app
            .stream_started
            .map(|start| start.elapsed().as_secs())
            .unwrap_or(0);
        let doing = match &app.activity {
            Some(tool) => format!("running {tool} "),
            None => "thinking ".to_string(),
        };
        let mut spans = vec![
            Span::styled(
                format!(" {frame_char} {doing}{elapsed}s "),
                Style::default().bg(bg).fg(ACCENT),
            ),
            Span::styled("· ", bar),
        ];
        spans.extend(status_spans(status, bg));
        spans.push(Span::styled("  (Esc to cancel) ", bar));
        Line::from(spans)
    } else if app.selection_mode {
        let mut spans = vec![Span::styled(
            " SELECT ",
            Style::default()
                .bg(ACCENT)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )];
        spans.extend(status_spans(status, bg));
        spans.push(Span::styled(
            "  ·  drag to copy  ·  Ctrl+O to resume scrolling ",
            bar,
        ));
        Line::from(spans)
    } else {
        let mut spans = status_spans(status, bg);
        spans.push(Span::styled("  ·  ? for help ", bar));
        Line::from(spans)
    };
    frame.render_widget(Paragraph::new(line).style(bar), area);
}

/// Renders the bordered multi-line input box and positions the cursor.
fn draw_input(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT))
        .title(Span::styled(
            " saya ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    // Empty input: show a dim placeholder and park the cursor at the start.
    if app.input.is_empty() {
        let hint = Paragraph::new(Line::from(Span::styled(
            "Ask about your data, or type / for commands",
            Style::default().fg(SECONDARY),
        )))
        .block(block);
        frame.render_widget(hint, area);
        frame.set_cursor_position((inner.x, inner.y));
        return;
    }
    let visible_rows = (inner.height as usize).max(1);
    let (cursor_line, cursor_col) = app.input.cursor_line_col();
    let first = cursor_line.saturating_sub(visible_rows.saturating_sub(1));
    let shown: Vec<Line> = app
        .input
        .lines()
        .into_iter()
        .skip(first)
        .map(highlight_input_line)
        .collect();
    frame.render_widget(Paragraph::new(Text::from(shown)).block(block), area);
    frame.set_cursor_position((
        inner.x + cursor_col as u16,
        inner.y + (cursor_line - first) as u16,
    ));
}

/// Styles one input line: a leading slash-command word in the accent color, or
/// SQL keywords in green. Char-preserving so the cursor stays aligned.
fn highlight_input_line(line: &str) -> Line<'static> {
    if line.is_empty() {
        return Line::from(String::new());
    }
    if line.starts_with('/') {
        let cmd_end = line.find(char::is_whitespace).unwrap_or(line.len());
        let (cmd, tail) = line.split_at(cmd_end);
        return Line::from(vec![
            Span::styled(
                cmd.to_string(),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::raw(tail.to_string()),
        ]);
    }
    let mut spans = Vec::new();
    let mut word_start = 0usize;
    let mut in_word: Option<bool> = None;
    for (idx, ch) in line.char_indices() {
        let is_word = ch.is_alphanumeric() || ch == '_';
        match in_word {
            Some(current) if current == is_word => {}
            Some(_) => {
                spans.push(styled_segment(&line[word_start..idx]));
                word_start = idx;
                in_word = Some(is_word);
            }
            None => in_word = Some(is_word),
        }
    }
    if word_start < line.len() {
        spans.push(styled_segment(&line[word_start..]));
    }
    Line::from(spans)
}

/// Styles a single word/non-word segment, greening SQL keywords.
fn styled_segment(segment: &str) -> Span<'static> {
    const KEYWORDS: &[&str] = &[
        "SELECT", "FROM", "WHERE", "JOIN", "LEFT", "RIGHT", "INNER", "OUTER", "ON", "GROUP", "BY",
        "ORDER", "LIMIT", "HAVING", "WITH", "AS", "AND", "OR", "NOT", "IN", "IS", "NULL", "LIKE",
        "DISTINCT", "COUNT", "SUM", "AVG", "MIN", "MAX", "DESC", "ASC", "UNION", "ALL",
    ];
    if KEYWORDS.contains(&segment.to_ascii_uppercase().as_str()) {
        Span::styled(segment.to_string(), Style::default().fg(SUCCESS))
    } else {
        Span::raw(segment.to_string())
    }
}

/// Renders the slash-command popup floating just above the input box.
fn draw_menu(frame: &mut Frame<'_>, menu: &Menu, input_area: Rect) {
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
        .border_style(Style::default().fg(ACCENT))
        .title(Span::styled(" commands ", Style::default().fg(ACCENT)));
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(Text::from(lines)).block(block), area);
}

/// Builds one popup row: the value plus a dimmed description, with the selected
/// row filled edge-to-edge in the accent color.
fn menu_row(candidate: &super::complete::Candidate, selected: bool, width: usize) -> Line<'static> {
    if selected {
        let label = match &candidate.description {
            Some(desc) => format!("{}  {desc}", candidate.value),
            None => candidate.value.clone(),
        };
        let padded = format!("{label:<width$}");
        return Line::from(Span::styled(
            padded,
            Style::default()
                .bg(ACCENT)
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
            Style::default().fg(SECONDARY),
        ));
    }
    Line::from(spans)
}

/// Formats a single transcript view line with lightweight markdown styling for assistant output.
fn markdown_spans(line: &str) -> Vec<Span<'static>> {
    let base = Style::default().fg(Color::White);
    let trimmed = line.trim_start();

    if let Some(rest) = trimmed
        .strip_prefix("### ")
        .or_else(|| trimmed.strip_prefix("## "))
        .or_else(|| trimmed.strip_prefix("# "))
    {
        let heading_style = base.add_modifier(Modifier::BOLD);
        inline_spans(rest, heading_style)
    } else if let Some(rest) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
    {
        let mut spans = vec![Span::styled("• ", base.fg(ACCENT))];
        spans.extend(inline_spans(rest, base));
        spans
    } else {
        inline_spans(line, base)
    }
}

/// Parses inline bold (`**bold**`) and inline code (`` `code` ``) formatting.
fn inline_spans(text: &str, base: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut plain_buf = String::new();
    let mut rem = text;

    while !rem.is_empty() {
        let pos_bold = rem.find("**");
        let pos_code = rem.find('`');

        match (pos_bold, pos_code) {
            (Some(b), Some(c)) if b < c => {
                if let Some(close_rel) = rem[b + 2..].find("**") {
                    plain_buf.push_str(&rem[..b]);
                    if !plain_buf.is_empty() {
                        spans.push(Span::styled(std::mem::take(&mut plain_buf), base));
                    }
                    let mid = &rem[b + 2..b + 2 + close_rel];
                    spans.push(Span::styled(
                        mid.to_string(),
                        base.add_modifier(Modifier::BOLD),
                    ));
                    rem = &rem[b + 2 + close_rel + 2..];
                } else {
                    plain_buf.push_str(&rem[..b + 2]);
                    rem = &rem[b + 2..];
                }
            }
            (Some(b), None) => {
                if let Some(close_rel) = rem[b + 2..].find("**") {
                    plain_buf.push_str(&rem[..b]);
                    if !plain_buf.is_empty() {
                        spans.push(Span::styled(std::mem::take(&mut plain_buf), base));
                    }
                    let mid = &rem[b + 2..b + 2 + close_rel];
                    spans.push(Span::styled(
                        mid.to_string(),
                        base.add_modifier(Modifier::BOLD),
                    ));
                    rem = &rem[b + 2 + close_rel + 2..];
                } else {
                    plain_buf.push_str(&rem[..b + 2]);
                    rem = &rem[b + 2..];
                }
            }
            (_pos_b, Some(c)) => {
                if let Some(close_rel) = rem[c + 1..].find('`') {
                    plain_buf.push_str(&rem[..c]);
                    if !plain_buf.is_empty() {
                        spans.push(Span::styled(std::mem::take(&mut plain_buf), base));
                    }
                    let mid = &rem[c + 1..c + 1 + close_rel];
                    spans.push(Span::styled(mid.to_string(), base.fg(CODE_COLOR)));
                    rem = &rem[c + 1 + close_rel + 1..];
                } else {
                    plain_buf.push_str(&rem[..c + 1]);
                    rem = &rem[c + 1..];
                }
            }
            (None, None) => {
                plain_buf.push_str(rem);
                rem = "";
            }
        }
    }

    if !plain_buf.is_empty() {
        spans.push(Span::styled(plain_buf, base));
    }

    if spans.is_empty() {
        vec![Span::styled(String::new(), base)]
    } else {
        spans
    }
}

#[cfg(test)]
mod markdown_tests {
    use super::*;
    use ratatui::style::Modifier;

    #[test]
    fn test_bold_text() {
        let spans = markdown_spans("**bold** text");
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].content.as_ref(), "bold");
        assert!(spans[0].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(spans[1].content.as_ref(), " text");
        assert!(!spans[1].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn test_inline_code() {
        let spans = markdown_spans("run `SELECT 1` now");
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content.as_ref(), "run ");
        assert_eq!(spans[1].content.as_ref(), "SELECT 1");
        assert_eq!(spans[1].style.fg, Some(CODE_COLOR));
        assert_eq!(spans[2].content.as_ref(), " now");
    }

    #[test]
    fn test_heading() {
        let spans = markdown_spans("# Heading");
        let combined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(combined, "Heading");
        assert!(!combined.contains('#'));
        for span in &spans {
            assert!(span.style.add_modifier.contains(Modifier::BOLD));
        }
    }

    #[test]
    fn test_bullet_item() {
        let spans = markdown_spans("- item");
        assert!(!spans.is_empty());
        assert_eq!(spans[0].content.as_ref(), "• ");
        assert_eq!(spans[0].style.fg, Some(ACCENT));
        let rest_combined: String = spans[1..].iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(rest_combined, "item");
    }

    #[test]
    fn test_plain_text() {
        let spans = markdown_spans("plain text with no markers");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content.as_ref(), "plain text with no markers");
    }

    #[test]
    fn test_unclosed_bold() {
        let spans = markdown_spans("unclosed **bold");
        let combined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(combined, "unclosed **bold");
        assert!(combined.contains("**"));
    }
}
