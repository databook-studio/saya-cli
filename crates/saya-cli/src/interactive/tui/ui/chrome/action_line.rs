//! The busy line's action text and its width budget.
//!
//! The action text and the bar's width arithmetic are one coupled cluster
//! (`CANCEL_HINT`, the tail width, `total_row_width`): they move together so
//! neither side dangles.

#[cfg(test)]
use super::super::theme::{secondary, status_bg};
#[cfg(test)]
use super::status::status_spans;
#[cfg(test)]
use crate::interactive::session_prompt::StatusView;
use crate::interactive::tui::types::App;
#[cfg(test)]
use ratatui::{style::Style, text::Span};
/// The bar's cancel hint, kept verbatim in one place: the busy line's width
/// budget subtracts exactly this, so a long action detail can never push it
/// off the bar.
pub(super) const CANCEL_HINT: &str = "  (Esc to cancel) ";

/// The smallest action room the bar ever grants: the full
/// `running bounded_sql_query: select region, count(*) from orders ` phrase
/// the suite pins (63 chars). A narrower terminal still renders the bar —
/// the tail overflows past the edge as today — but the action never sheds a
/// pinned target to chase a width the tail already exceeds.
pub(super) const MIN_ACTION_ROOM: usize = 63;

/// The fewest detail chars worth showing before the ellipsis. Below this the
/// truncated target is noise, so the line falls back to today's bare
/// `running {tool}` — the honest degradation, never a pushed-off bar.
const MIN_DETAIL_CHARS: usize = 8;

/// The busy line's action words: `thinking` while no tool runs, otherwise
/// `running {tool}` plus the call's target through the shared detail seam —
/// the path for a write, the program for a command, the SQL for a query.
/// Names the action and its target, never a motive. `room` is the char
/// budget for the whole `running … ` phrase: the caller passes what fits
/// beside the painted tail, and only the detail truncates — the tool name,
/// the elapsed time, and the cancel hint never move.
pub(super) fn action_text(
    activity: Option<&str>,
    running: Option<(String, serde_json::Value)>,
    room: usize,
) -> String {
    let Some(tool) = activity.map(str::to_string) else {
        return "thinking ".to_string();
    };
    let bare = format!("running {tool} ");
    // The arguments ride the transcript's pending-tool buffer — the one place
    // "what is running" is already recorded — newest open call of this name.
    let detail = running.as_ref().and_then(|(name, arguments)| {
        (name == &tool)
            .then(|| crate::interactive::tui::stream_events::tool_call_detail(name, arguments))
            .flatten()
    });
    let Some(detail) = detail else { return bare };
    let detail = one_line(&detail);
    let full = format!("running {tool}: {detail} ");
    if full.chars().count() <= room.max(bare.chars().count()) {
        return full;
    }
    let head = format!("running {tool}: ");
    // Below this the truncated target is noise: fall back to the bare tool
    // name rather than show a sliver.
    let budget = room.saturating_sub(head.chars().count() + 2);
    if budget < MIN_DETAIL_CHARS {
        return bare;
    }
    format!("{head}{}… ", head_chars(&detail, budget))
}

/// The newest open pending call's name and arguments: the one place "what is
/// running" is already recorded, read at paint time. `None` when nothing is
/// open — the bar falls back to the bare tool name.
pub(super) fn running_call(app: &App) -> Option<(String, serde_json::Value)> {
    app.transcript
        .newest_open_tool()
        .map(|(name, arguments)| (name.to_string(), arguments.clone()))
}

/// One line: the detail seam is single-line today, but the bar must stay one
/// line even if a future detail is not.
fn one_line(detail: &str) -> String {
    detail.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The first `budget` chars — char-boundary safe, never splitting mid-grapheme.
fn head_chars(detail: &str, budget: usize) -> String {
    detail.chars().take(budget).collect()
}

/// The full busy-row width with the given action phrase: spinner, action,
/// elapsed, separator, tail, cancel hint — the same sum `draw_status`
/// budgets from, shared so the two cannot drift.
pub(super) fn total_row_width(
    frame_width: usize,
    action: &str,
    elapsed_width: usize,
    tail_width: usize,
) -> usize {
    frame_width + action.chars().count() + elapsed_width + tail_width
}

/// Test seams for the truncation budget: the bar's own row width and tail
/// width, so the suite asserts the budget the renderer draws from — not the
/// frame-clipped pixels, which today's over-wide tail already overflows.
#[cfg(test)]
pub(crate) fn tail_width_for_test(status: &StatusView) -> usize {
    let bar = Style::default().bg(status_bg()).fg(secondary());
    let bg = status_bg();
    status_spans(status, bg)
        .iter()
        .map(|span| span.width())
        .sum::<usize>()
        + Span::styled(CANCEL_HINT, bar).width()
        + Span::styled("· ", bar).width()
}

/// Test seams for the truncation budget: the bar's own row width and tail
/// width, so the suite asserts the budget the renderer draws from — not the
/// frame-clipped pixels, which today's over-wide tail already overflows.
#[cfg(test)]
pub(crate) fn total_row_width_for_test(
    activity: Option<&str>,
    running: Option<(String, serde_json::Value)>,
    elapsed: u64,
    status: &StatusView,
) -> usize {
    let tail = tail_width_for_test(status);
    let elapsed_width = format!("{elapsed}s ").chars().count();
    let frame_width = 3;
    total_row_width(
        frame_width,
        &action_text(activity, running, usize::MAX),
        elapsed_width,
        tail,
    )
}
