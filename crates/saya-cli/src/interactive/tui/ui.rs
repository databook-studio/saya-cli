//! Rendering for the TUI: transcript, status bar, input box, and the slash
//! popup. Kept separate from the event loop so styling can evolve on its own.

use super::transcript::BlockKind;
use super::{App, Menu};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, BorderType, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation,
        ScrollbarState,
    },
};

/// Largest number of rows shown in the slash-command popup.
const MAX_MENU_ROWS: usize = 8;

/// Accent color used for the input frame, the popup, and user turns.
const ACCENT: Color = Color::Cyan;

/// Maps a transcript block kind to its display style.
fn kind_style(kind: BlockKind) -> Style {
    match kind {
        BlockKind::User => Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        BlockKind::Assistant => Style::default().fg(Color::White),
        BlockKind::System => Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
        BlockKind::Error => Style::default()
            .fg(Color::LightRed)
            .add_modifier(Modifier::BOLD),
        BlockKind::Tool => Style::default().fg(Color::Green),
    }
}

/// Draws one frame: transcript (fills), status bar, input box, popup overlay.
pub(super) fn draw(frame: &mut Frame<'_>, app: &App, status: &str) {
    let input_height = (app.input_rows() as u16) + 2;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(input_height),
        ])
        .split(frame.area());

    draw_transcript(frame, app, chunks[0]);
    draw_status(frame, app, status, chunks[1]);
    draw_input(frame, app, chunks[2]);
    if let Some(menu) = &app.menu {
        draw_menu(frame, menu, chunks[2]);
    }
    if let Some(pending) = &app.pending_approval {
        draw_approval(frame, &pending.tool, frame.area());
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
        "/sessions    resume picker  ·  ? or F1  toggle this help",
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

/// Draws the centered tool-approval modal.
fn draw_approval(frame: &mut Frame<'_>, tool: &str, screen: Rect) {
    let width = screen.width.clamp(30, 60);
    let height = 5u16.min(screen.height);
    let area = Rect {
        x: screen.x + (screen.width.saturating_sub(width)) / 2,
        y: screen.y + (screen.height.saturating_sub(height)) / 2,
        width,
        height,
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Yellow))
        .title(Span::styled(
            " approval required ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    let body = Text::from(vec![
        Line::from(format!("Run tool `{tool}`?")),
        Line::from(""),
        Line::from(Span::styled(
            "[y] allow    [n] deny",
            Style::default().add_modifier(Modifier::BOLD),
        )),
    ]);
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(body).block(block), area);
}

/// Spinner frames shown while an agent request is streaming.
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Renders the visible, soft-wrapped transcript lines with per-kind styling,
/// plus a scrollbar when the content overflows.
fn draw_transcript(frame: &mut Frame<'_>, app: &App, area: Rect) {
    app.viewport.set((area.width, area.height));
    let width = area.width as usize;
    let height = area.height as usize;
    let lines: Vec<Line> = app
        .transcript
        .view(width, height)
        .into_iter()
        .map(|(kind, text)| Line::from(Span::styled(text, kind_style(kind))))
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

/// Renders the status bar as a filled accent-tinted strip, with a spinner and
/// hint while an agent request is streaming.
fn draw_status(frame: &mut Frame<'_>, app: &App, status: &str, area: Rect) {
    let bar = Style::default().bg(Color::Rgb(38, 40, 54)).fg(Color::Gray);
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
        Line::from(vec![
            Span::styled(
                format!(" {frame_char} {doing}{elapsed}s "),
                Style::default().bg(Color::Rgb(38, 40, 54)).fg(ACCENT),
            ),
            Span::styled(format!("· {status}  (Esc to cancel) "), bar),
        ])
    } else {
        Line::from(Span::styled(format!(" {status}  ·  ? for help "), bar))
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
            Style::default().fg(Color::DarkGray),
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
        Span::styled(segment.to_string(), Style::default().fg(Color::Green))
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
            Style::default().fg(Color::DarkGray),
        ));
    }
    Line::from(spans)
}
