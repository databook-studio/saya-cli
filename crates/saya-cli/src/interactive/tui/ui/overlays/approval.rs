//! The docked approval panel: what will run, and the answers that decide it.

use super::super::theme::{accent, secondary, warning};
use crate::interactive::tui::wrap::wrap_cells;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};

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
        wrap_cells(&crate::grant_token::session_answers_line(grant), inner).len() as u16;
    match detail {
        Some(d) => (6 + body_row_count(d, inner) as u16 + answers_rows - 1).min(24),
        None => (4 + answers_rows).min(16),
    }
}

/// How many rows the body paints at `inner` columns — the length of the
/// pre-wrapped rows [`detail_rows`] will draw, from the shared cell-aware,
/// grapheme-safe wrapper the transcript flattens with. The panel decides fit
/// and the scroll end-stop with this count and then paints exactly those
/// rows without ratatui's own wrap, so the count and the paint are the same
/// thing by construction: a wide body cannot be counted short of what it
/// draws, and the end-stop cannot strand a row it never knew about
/// (re-audit R01).
fn body_row_count(body: &str, inner: usize) -> usize {
    body.lines().map(|line| wrap_cells(line, inner).len()).sum()
}

/// The body pre-wrapped into styled visual rows — one `Line` per row the
/// panel paints, the fact's style carried from its logical line (indented
/// lines in the accent, everything else bold). The text is verbatim:
/// clipping happens at the region edge — the rows themselves are never
/// rewritten or summarised.
fn detail_rows(body: &str, inner: usize) -> Vec<Line<'static>> {
    body.lines()
        .flat_map(|line| {
            let style = if line.starts_with("  ") {
                Style::default().fg(accent())
            } else {
                Style::default().add_modifier(Modifier::BOLD)
            };
            wrap_cells(line, inner)
                .into_iter()
                .map(move |row| Line::from(Span::styled(row, style)))
        })
        .collect()
}

/// The shared answers line pre-wrapped the same way, bold — the same wording
/// the terminal prompt appends, measured and painted from one list.
fn answer_rows(answers: &str, inner: usize) -> Vec<Line<'static>> {
    wrap_cells(answers, inner)
        .into_iter()
        .map(|row| {
            Line::from(Span::styled(
                row,
                Style::default().add_modifier(Modifier::BOLD),
            ))
        })
        .collect()
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

/// One bordered paragraph of pre-wrapped rows — the panel's shape when
/// everything fits.
fn paint(frame: &mut Frame<'_>, lines: Vec<Line<'static>>, block: Block<'static>, area: Rect) {
    frame.render_widget(Paragraph::new(Text::from(lines)).block(block), area);
}

/// Draws the tool-approval panel into the given area. The body is the shared
/// fact text — the same lines the terminal prompt renders
/// (`approval_facts::call_facts`) — drawn verbatim, so the modal cannot
/// state different facts for the same call; the answers line is the shared
/// three-answer text — with the offered token when one exists, two answers
/// and the reason when not.
///
/// The body is pre-wrapped into the exact visual rows the paint draws (the
/// shared cell-aware wrapper), so the row count *is* the painted row count.
/// The answers' rows are reserved out of the panel height. When the body
/// fits under that reservation, the panel paints everything flat; when it
/// does not, the detail clips at the reservation, the last visible detail
/// row names how much was withheld, and the answers still paint immediately
/// after it. `scroll` then moves a verbatim window of the wrapped facts
/// through the detail region — clamped here against the live geometry, so it
/// cannot run past either end, and left alone entirely when the body fits —
/// while the answers row stays pinned: the offset moves the detail region
/// only.
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
        let inner = area.width.saturating_sub(2).max(1) as usize;
        let mut lines = vec![Line::from(format!("Run tool `{tool}`?")), Line::from("")];
        lines.extend(answer_rows(&answers, inner));
        paint(frame, lines, approval_block(), area);
        return;
    };

    let inner_w = area.width.saturating_sub(2).max(1) as usize;
    let answer_lines = answer_rows(&answers, inner_w);
    let rows = detail_rows(body, inner_w);
    let inner_h = area.height.saturating_sub(2) as usize;
    // The reservation: the blank separator plus the answers' wrapped rows.
    let budget = inner_h.saturating_sub(answer_lines.len() + 1);
    if rows.len() <= budget {
        // Everything fits — the flat paragraph the panel has always painted,
        // byte for byte: the count is the length of these very rows, so a
        // "fits" here cannot clip the answers below them.
        let mut lines = rows;
        lines.push(Line::from(""));
        lines.extend(answer_lines);
        paint(frame, lines, approval_block(), area);
        return;
    }

    // Overflow: the detail clips at the answers' reservation and the cut is
    // labelled. The rows are already wrapped, so the scroll offset selects a
    // verbatim window of exactly what was counted, and the region edge does
    // the clipping.
    let block = approval_block();
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let budget = budget.min(inner.height as usize) as u16;
    let visible = budget as usize;
    // The offset cannot run past either end: at the maximum the final fact
    // rows fill the region and nothing is withheld; a body that fits never
    // reaches this branch, so it cannot scroll at all. The end-stop counts
    // the same rows the paint draws, so maximum scroll lands on the last
    // painted row.
    let max_offset = rows.len().saturating_sub(visible);
    let offset = scroll.min(max_offset);

    frame.render_widget(
        Paragraph::new(Text::from(rows[offset..offset + visible].to_vec())),
        Rect::new(inner.x, inner.y, inner.width, budget),
    );

    // The last visible detail row names the withheld material — until the
    // scroll reaches the end, where the final facts fill the region and
    // the marker is gone.
    let withheld = rows
        .len()
        .saturating_sub(offset + visible.saturating_sub(1));
    if budget >= 1 && offset < max_offset {
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
    let start = (budget + 1).min(inner.height.saturating_sub(answer_lines.len() as u16));
    frame.render_widget(
        Paragraph::new(Text::from(answer_lines)),
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
#[cfg(test)]
#[path = "approval_unicode_tests.rs"]
mod unicode_tests;
