//! Input box rendering and SQL keyword highlighting.

use super::theme::{accent, secondary, success};
use crate::interactive::tui::types::App;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Paragraph},
};

/// Renders the bordered multi-line input box and positions the cursor.
///
/// Wrapping is pre-computed (char count, see the limitation note on
/// [`crate::interactive::tui::input::wrap`]) and the `Paragraph` is rendered
/// **without** `.wrap()`, so the pre-split visual lines are authoritative and
/// the cursor — mapped from the same split — can never disagree with what is
/// on screen.
/// Hint naming what Enter does with a single-line draft.
const SEND_HINT: &str = " Enter sends ";
/// Hint for a multiline draft: Enter sends every line, Alt+Enter adds one.
const SEND_ALL_HINT: &str = " Enter sends all lines · Alt+Enter new line ";

pub(in crate::interactive::tui) fn draw_input(frame: &mut Frame<'_>, app: &App, area: Rect) {
    // The hint rides the bottom border (a `Block` title), so the box keeps
    // the `input_rows + 2` height `ui::draw` budgets: no content row grows.
    let send_hint = if app.input.text().contains('\n') {
        SEND_ALL_HINT
    } else {
        SEND_HINT
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(accent()))
        .title(Span::styled(
            " saya ",
            Style::default().fg(accent()).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Line::from(Span::styled(
            send_hint,
            Style::default().fg(secondary()),
        )));
    let inner = block.inner(area);
    // Empty input: show a dim placeholder and park the cursor at the start.
    if app.input.is_empty() {
        let hint = Paragraph::new(Line::from(Span::styled(
            "Ask about your data, or type / for commands",
            Style::default().fg(secondary()),
        )))
        .block(block);
        frame.render_widget(hint, area);
        frame.set_cursor_position((inner.x, inner.y));
        return;
    }
    let width = inner.width.max(1) as usize;
    // Pre-split every logical line into visual lines so the Paragraph renders
    // the wrap (it has no `.wrap()`) and the cursor shares the same breaks.
    let visual: Vec<Line> = app
        .input
        .wrapped_lines(width)
        .into_iter()
        .map(|s| highlight_input_line(&s))
        .collect();
    let (cur_row, cur_col) = app.input.cursor_visual(width);

    let visible_rows = (inner.height as usize).max(1);
    // Vertical scroll over visual rows: keep the cursor's row in view by
    // starting the window at the row that shows it near the bottom.
    let first = cur_row.saturating_sub(visible_rows.saturating_sub(1));
    let shown: Vec<Line> = visual.into_iter().skip(first).collect();
    frame.render_widget(Paragraph::new(Text::from(shown)).block(block), area);
    frame.set_cursor_position((
        inner.x + cur_col.min(width) as u16,
        inner.y + (cur_row - first) as u16,
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
                Style::default().fg(accent()).add_modifier(Modifier::BOLD),
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
        Span::styled(segment.to_string(), Style::default().fg(success()))
    } else {
        Span::raw(segment.to_string())
    }
}
