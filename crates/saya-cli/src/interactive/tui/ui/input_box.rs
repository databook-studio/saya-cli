//! Input box rendering and SQL keyword highlighting.

use super::theme::{ACCENT, SECONDARY, SUCCESS};
use crate::interactive::tui::types::App;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Paragraph},
};

/// Renders the bordered multi-line input box and positions the cursor.
pub(super) fn draw_input(frame: &mut Frame<'_>, app: &App, area: Rect) {
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
