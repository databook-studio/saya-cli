//! The home screen shown before the first conversation turn: one rounded
//! box sized to its content and anchored at the top of the transcript area,
//! with a Welcome column and a Getting-started/Status column. Narrow
//! terminals stack the two columns; a short terminal drops the mascot
//! first, then Getting started, never Status.

mod lines;
mod state;

use super::super::theme::accent;
use crate::interactive::session_prompt::StatusView;
use crate::interactive::tui::types::App;
use lines::{
    left_lines, left_lines_without_mascot, mode_text, right_lines, rule_line, status_block,
};
use ratatui::{
    Frame,
    buffer::Buffer,
    layout::{Alignment, Rect},
    style::Style,
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Paragraph, Widget, Wrap},
};

/// Below this terminal width the two columns stack vertically: side by side
/// each column would be too narrow for the status lines to read.
const STACK_BELOW_WIDTH: u16 = 70;

/// Two cells of horizontal padding inside each column, so text touches
/// neither the box borders nor the vertical rule.
const COLUMN_PAD: u16 = 2;

/// The rounded box: accent border, version on the title border.
fn home_block() -> Block<'static> {
    let title = format!(" saya v{} ", env!("CARGO_PKG_VERSION"));
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(accent()))
        .title(Span::styled(title, Style::default().fg(accent())))
}

/// The wrapped height of a block of lines at `width`, capped at `max`.
/// Renders into a scratch buffer so the count matches the paragraph the
/// frame paints.
fn wrapped_height(lines: &[Line<'static>], width: u16, max: u16) -> u16 {
    if width == 0 || max == 0 || lines.is_empty() {
        return 0;
    }
    let area = Rect::new(0, 0, width, max);
    let mut buffer = Buffer::empty(area);
    Paragraph::new(Text::from(lines.to_vec()))
        .wrap(Wrap { trim: false })
        .render(area, &mut buffer);
    let width = width as usize;
    buffer
        .content
        .chunks(width)
        .enumerate()
        .filter(|(_, cells)| cells.iter().any(|cell| cell.symbol() != " "))
        .map(|(row, _)| row as u16 + 1)
        .max()
        .unwrap_or(0)
}

/// Insets a column by [`COLUMN_PAD`] on both sides.
fn padded(rect: Rect) -> Rect {
    Rect {
        x: rect.x.saturating_add(COLUMN_PAD),
        y: rect.y,
        width: rect.width.saturating_sub(COLUMN_PAD * 2),
        height: rect.height,
    }
}

/// Draws the home screen: one content-sized rounded box at the top of the
/// pane. No footer line — the status bar already shows `? for help`.
pub(in crate::interactive::tui) fn draw_empty_state(
    frame: &mut Frame<'_>,
    app: &App,
    status: &StatusView,
    area: Rect,
    workspace_bound: Option<bool>,
) {
    if area.width < 8 || area.height < 3 {
        return;
    }
    let provider = app.runtime.resolved.ai.provider.as_str();
    let mode = mode_text(status);
    if area.width < STACK_BELOW_WIDTH {
        draw_stacked(frame, app, status, area, workspace_bound, provider, mode);
    } else {
        draw_columns(frame, app, status, area, workspace_bound, provider, mode);
    }
}

/// Two columns split by a vertical rule. The mascot drops first, then the
/// Getting-started block; Status stays.
fn draw_columns(
    frame: &mut Frame<'_>,
    app: &App,
    status: &StatusView,
    area: Rect,
    workspace_bound: Option<bool>,
    provider: &str,
    mode: &str,
) {
    let inner_width = area.width - 2;
    let left_width = (u32::from(inner_width) * 40 / 100) as u16;
    let right_width = inner_width.saturating_sub(left_width + 1);
    let left_text = left_width.saturating_sub(COLUMN_PAD * 2);
    let right_text = right_width.saturating_sub(COLUMN_PAD * 2);

    let left_full = left_lines(status, provider, mode);
    let left_short = left_lines_without_mascot(status, provider, mode);
    let right_full = right_lines(app, workspace_bound, right_text as usize);
    let right_status = status_block(app, workspace_bound);

    let candidates = [
        (&left_full, &right_full),
        (&left_short, &right_full),
        (&left_short, &right_status),
    ];
    let mut chosen = None;
    for (left, right) in candidates {
        let left_height = wrapped_height(left, left_text, area.height);
        let right_height = wrapped_height(right, right_text, area.height);
        let box_height = 1 + left_height.max(right_height) + 2;
        if box_height <= area.height {
            chosen = Some((left, right, box_height));
            break;
        }
    }
    let (left, right, box_height) = chosen.unwrap_or_else(|| {
        let left_height = wrapped_height(&left_short, left_text, area.height);
        let right_height = wrapped_height(&right_status, right_text, area.height);
        (
            &left_short,
            &right_status,
            (1 + left_height.max(right_height) + 2).min(area.height),
        )
    });

    let box_area = Rect {
        height: box_height,
        ..area
    };
    let block = home_block();
    let inner = block.inner(box_area);
    frame.render_widget(block, box_area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let divider_x = inner.x + left_width;
    frame.render_widget(
        Block::default()
            .borders(Borders::LEFT)
            .border_style(Style::default().fg(accent())),
        Rect {
            x: divider_x,
            y: inner.y,
            width: 1,
            height: inner.height,
        },
    );

    let content = Rect {
        y: inner.y + 1,
        height: inner.height.saturating_sub(1),
        ..inner
    };
    render_centered(
        frame,
        left,
        padded(Rect {
            width: left_width,
            ..content
        }),
    );
    render_wrapped(
        frame,
        right,
        padded(Rect {
            x: divider_x + 1,
            width: right_width,
            ..content
        }),
    );
}

/// The two columns stacked, separated by a horizontal rule. The same drop
/// order applies: mascot first, then Getting started, never Status.
fn draw_stacked(
    frame: &mut Frame<'_>,
    app: &App,
    status: &StatusView,
    area: Rect,
    workspace_bound: Option<bool>,
    provider: &str,
    mode: &str,
) {
    let text_width = area.width.saturating_sub(2 + COLUMN_PAD * 2);
    let left_full = left_lines(status, provider, mode);
    let left_short = left_lines_without_mascot(status, provider, mode);
    let right_full = right_lines(app, workspace_bound, text_width as usize);
    let right_status = status_block(app, workspace_bound);

    let candidates = [
        (&left_full, &right_full),
        (&left_short, &right_full),
        (&left_short, &right_status),
    ];
    let mut chosen = None;
    for (left, right) in candidates {
        let left_height = wrapped_height(left, text_width, area.height);
        let right_height = wrapped_height(right, text_width, area.height);
        let box_height = 1 + left_height + 1 + right_height + 2;
        if box_height <= area.height {
            chosen = Some((left, right, left_height, right_height, box_height));
            break;
        }
    }
    let (left, right, left_height, right_height, box_height) = chosen.unwrap_or_else(|| {
        let left_height = wrapped_height(&left_short, text_width, area.height);
        let right_height = wrapped_height(&right_status, text_width, area.height);
        (
            &left_short,
            &right_status,
            left_height,
            right_height,
            (1 + left_height + 1 + right_height + 2).min(area.height),
        )
    });

    let box_area = Rect {
        height: box_height,
        ..area
    };
    let block = home_block();
    let inner = block.inner(box_area);
    frame.render_widget(block, box_area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let content_y = inner.y + 1;
    render_centered(
        frame,
        left,
        padded(Rect {
            x: inner.x,
            y: content_y,
            width: inner.width,
            height: left_height,
        }),
    );
    frame.render_widget(
        Paragraph::new(rule_line(inner.width as usize)),
        Rect {
            x: inner.x,
            y: content_y + left_height,
            width: inner.width,
            height: 1,
        },
    );
    render_wrapped(
        frame,
        right,
        padded(Rect {
            x: inner.x,
            y: content_y + left_height + 1,
            width: inner.width,
            height: right_height,
        }),
    );
}

/// Centres the lines horizontally; the column is top-aligned. Wrapping is
/// on so a long model name in the status line cannot clip its mode.
fn render_centered(frame: &mut Frame<'_>, lines: &[Line<'static>], area: Rect) {
    frame.render_widget(
        Paragraph::new(Text::from(lines.to_vec()))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false }),
        area,
    );
}

/// Left-aligned and wrapped: a readiness line with a long doctor detail
/// wraps rather than clipping its remedy away.
fn render_wrapped(frame: &mut Frame<'_>, lines: &[Line<'static>], area: Rect) {
    frame.render_widget(
        Paragraph::new(Text::from(lines.to_vec())).wrap(Wrap { trim: false }),
        area,
    );
}

#[cfg(test)]
#[path = "empty_state_tests.rs"]
mod tests;
