//! The docked approval panel: what will run, and the answers that decide it.

use super::super::theme::{accent, secondary, warning};
use crate::interactive::tui::input::wrap::wrap_line;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
};

/// Wrapped rows one text needs at `inner` columns — the char arithmetic the
/// panel's height has always used, shared so the height and the paint agree
/// on the answers' reservation.
fn wrapped_rows(text: &str, inner: usize) -> usize {
    text.lines()
        .map(|line| line.chars().count().max(1).div_ceil(inner))
        .sum()
}

/// Computes the required vertical height (in rows) for the approval panel.
/// The answers line wraps like any other line, so a long offered token
/// claims its rows instead of clipping. The per-call fact bodies
/// (`approval_facts`) run longer than the old SQL-only detail, so the cap
/// follows them: an interpreter `run_program` prompt carries the no-euphemism
/// warning, and clipping a containment fact is worse than a taller panel.
pub(in crate::interactive::tui) fn approval_height(
    detail: Option<&str>,
    grant: Option<&str>,
    width: u16,
) -> u16 {
    let inner = width.saturating_sub(2).max(1) as usize;
    let answers_rows =
        wrapped_rows(&crate::grant_token::session_answers_line(grant), inner).max(1) as u16;
    match detail {
        Some(d) => (6 + wrapped_rows(d, inner) as u16 + answers_rows - 1).min(24),
        None => (4 + answers_rows).min(16),
    }
}

/// The body's lines, verbatim: indented fact lines in the accent, everything
/// else bold. Clipping happens at the region edge — the text itself is never
/// rewritten or summarised.
fn detail_lines(body: &str) -> Vec<Line<'static>> {
    body.lines()
        .map(|line| {
            Line::from(Span::styled(
                line.to_string(),
                if line.starts_with("  ") {
                    Style::default().fg(accent())
                } else {
                    Style::default().add_modifier(Modifier::BOLD)
                },
            ))
        })
        .collect()
}

/// How many rows the body really wraps into at `inner` columns — the same
/// word-aware arithmetic the input box counts with. The panel decides fit
/// and the scroll end-stop with this, not [`wrapped_rows`]: the char
/// arithmetic above undercounts a prose line that ratatui's paragraph wraps
/// into several rows, leaving such a body clipped and unlabelled, or
/// stranding its final rows below the end-stop — E1's defect from the other
/// side. The paint is ratatui's own wrap; this counter is the codebase's
/// canonical model of it.
fn body_row_count(body: &str, inner: usize) -> usize {
    body.lines().map(|line| wrap_line(line, inner).len()).sum()
}

/// The shared answers line, bold — the same wording the terminal prompt appends.
fn answers_row(answers: &str) -> Line<'static> {
    Line::from(Span::styled(
        answers.to_string(),
        Style::default().add_modifier(Modifier::BOLD),
    ))
}

fn approval_block() -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(warning()))
        .title(Span::styled(
            " approval required ",
            Style::default().fg(warning()).add_modifier(Modifier::BOLD),
        ))
}

/// One bordered wrapped paragraph — the panel's shape when everything fits.
fn paint(frame: &mut Frame<'_>, lines: Vec<Line<'static>>, block: Block<'static>, area: Rect) {
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

/// Draws the tool-approval panel into the given area. The body is the shared
/// fact text — the same lines the terminal prompt renders
/// (`approval_facts::call_facts`) — drawn verbatim, so the modal cannot
/// state different facts for the same call; the answers line is the shared
/// three-answer text — with the offered token when one exists, two answers
/// and the reason when not.
///
/// The answers' rows are reserved out of the panel height. When the wrapped
/// body fits under that reservation, the panel paints exactly as it always
/// did; when it does not, the detail clips at the reservation, the last
/// visible detail row names how much was withheld, and the answers still
/// paint immediately after it. `scroll` then moves a verbatim window of the
/// wrapped facts through the detail region — clamped here against the live
/// geometry, so it cannot run past either end, and left alone entirely when
/// the body fits — while the answers row stays pinned: the offset moves the
/// detail region only.
pub(in crate::interactive::tui) fn draw_approval(
    frame: &mut Frame<'_>,
    tool: &str,
    detail: Option<&str>,
    grant: Option<&str>,
    scroll: usize,
    area: Rect,
) {
    let answers = crate::grant_token::session_answers_line(grant);
    let Some(body) = detail else {
        // No per-call facts: the tool name, a blank, the answers.
        paint(
            frame,
            vec![
                Line::from(format!("Run tool `{tool}`?")),
                Line::from(""),
                answers_row(&answers),
            ],
            approval_block(),
            area,
        );
        return;
    };

    let inner_w = area.width.saturating_sub(2).max(1) as usize;
    let answers_rows = wrapped_rows(&answers, inner_w).max(1);
    let detail_rows = body_row_count(body, inner_w);
    let inner_h = area.height.saturating_sub(2) as usize;
    // The reservation: the blank separator plus the answers' wrapped rows.
    let budget = inner_h.saturating_sub(answers_rows + 1);
    if detail_rows <= budget {
        // Everything fits — the flat paragraph the panel has always painted,
        // byte for byte.
        let mut lines = detail_lines(body);
        lines.push(Line::from(""));
        lines.push(answers_row(&answers));
        paint(frame, lines, approval_block(), area);
        return;
    }

    // Overflow: the detail clips at the answers' reservation and the cut is
    // labelled. The body text goes to the paragraph whole; the clamped
    // scroll offset moves a verbatim window of the wrapped facts through
    // the region, and the region edge does the clipping.
    let block = approval_block();
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let budget = budget.min(inner.height as usize) as u16;
    let visible = budget as usize;
    // The offset cannot run past either end: at the maximum the final fact
    // rows fill the region and nothing is withheld; a body that fits never
    // reaches this branch, so it cannot scroll at all. The end-stop counts
    // the rows the body really wraps into (see `body_row_count`).
    let max_offset = detail_rows.saturating_sub(visible);
    let offset = scroll.min(max_offset) as u16;

    frame.render_widget(
        Paragraph::new(Text::from(detail_lines(body)))
            .wrap(Wrap { trim: false })
            .scroll((offset, 0)),
        Rect::new(inner.x, inner.y, inner.width, budget),
    );

    // The last visible detail row names the withheld material — until the
    // scroll reaches the end, where the final facts fill the region and
    // the marker is gone.
    let withheld = detail_rows.saturating_sub(offset as usize + visible.saturating_sub(1));
    if budget >= 1 && usize::from(offset) < max_offset {
        let marker = Rect::new(inner.x, inner.y + budget - 1, inner.width, 1);
        frame.render_widget(Clear, marker);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!("… {withheld} more lines hidden"),
                Style::default().fg(secondary()),
            ))),
            marker,
        );
    }

    // Blank separator, then the answers inside their reservation —
    // immediately after the clipped detail, never pinned to the floor.
    let start = (budget + 1).min(inner.height.saturating_sub(answers_rows as u16));
    frame.render_widget(
        Paragraph::new(answers_row(&answers)).wrap(Wrap { trim: false }),
        Rect::new(
            inner.x,
            inner.y + start,
            inner.width,
            inner.height.saturating_sub(start),
        ),
    );
}

#[cfg(test)]
#[path = "approval_scroll_tests.rs"]
mod scroll_tests;
#[cfg(test)]
#[path = "approval_tests.rs"]
mod tests;
