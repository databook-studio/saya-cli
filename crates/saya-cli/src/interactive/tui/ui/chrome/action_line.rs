//! The busy line's action text and its width budget.
//!
//! The action text and the bar's width arithmetic are one coupled cluster
//! (`CANCEL_HINT`, the separator, the busy-row plan): they move together so
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
/// The bar's cancel hint, kept verbatim in one place: the busy row reserves
/// exactly this before anything else is sized, so no other span can push it
/// off the bar.
pub(super) const CANCEL_HINT: &str = "  (Esc to cancel) ";

/// The busy row's separator between the action phrase and the status detail.
pub(super) const SEPARATOR: &str = "· ";

/// The fewest detail chars worth showing before the ellipsis. Below this the
/// truncated target is noise, so the line falls back to the bare
/// `running {tool}` — the honest degradation, never a pushed-off bar.
const MIN_DETAIL_CHARS: usize = 8;

/// The bare action phrase: `running {tool}` while a tool runs, `thinking`
/// before any does. One builder for the phrase and its width
/// ([`bare_action_width`]) so the two cannot drift.
fn bare_text(activity: Option<&str>) -> String {
    match activity {
        Some(tool) => format!("running {tool} "),
        None => "thinking ".to_string(),
    }
}

/// The bare phrase's width — the floor the busy row plans against: what the
/// row paints when the detail is absent or the room cannot afford one.
pub(super) fn bare_action_width(activity: Option<&str>) -> usize {
    bare_text(activity).chars().count()
}

/// The busy row's width plan: how the frame's columns are handed out,
/// decided by reserving the stop affordance first and shedding in the
/// design's hierarchy order.
///
/// The cancel hint and the fixed chrome it rides with (spinner frame, elapsed
/// time, separator) come off the frame width first — the hint is the one
/// affordance naming how to stop a running request, so it is never what the
/// row drops (audit F06: it painted last and paid for every other span's
/// overflow). What remains is spent in the shedding hierarchy:
///
/// 1. the routine status detail sheds, trailing segment first, before the
///    action truncates — the context line's drop-rank scale is the design's
///    precedent: routine detail yields before anything load-bearing;
/// 2. the action detail truncates next — it is the elastic part, falling
///    back through [`action_text`] to the bare tool name when the room
///    cannot afford one;
/// 3. the new-activity notice sheds only when even the bare action cannot
///    fit beside it.
///
/// The old floor (`MIN_ACTION_ROOM`, 63) is dropped rather than clamped:
/// with the hint reserved first and the room measured against the real
/// frame, the floor's one remaining effect was to defy the shed and overrun
/// the row — the defect itself. Where the frame cannot afford even the
/// pinned 63-char target, [`action_text`]'s own floor (`MIN_DETAIL_CHARS`)
/// falls back to the bare tool name, the honest degradation the design
/// already owns.
pub(super) struct BusyRowPlan {
    /// Whether the new-activity notice paints this frame. When it does, it
    /// still leads the row.
    pub lead_painted: bool,
    /// Char budget for the whole `running … ` phrase.
    pub action_room: usize,
    /// How many leading status-detail segments may paint; the rest shed.
    pub status_segments: usize,
}

/// Reserves the hint and its fixed chrome off the frame width, then spends
/// what is left down the shedding hierarchy. The caller passes the status
/// detail's per-segment widths so the arithmetic stays here, beside the
/// constants the row is measured with.
pub(super) fn busy_row_plan(
    frame_width: usize,
    lead_width: usize,
    spinner_width: usize,
    elapsed_width: usize,
    segment_widths: &[usize],
    full_action_width: usize,
    bare_action_width: usize,
) -> BusyRowPlan {
    let chrome =
        spinner_width + elapsed_width + SEPARATOR.chars().count() + CANCEL_HINT.chars().count();
    let base = frame_width.saturating_sub(chrome);
    // Everything fits: paint all of it, notice included.
    if lead_width + full_action_width + segment_widths.iter().sum::<usize>() <= base {
        return BusyRowPlan {
            lead_painted: lead_width > 0,
            action_room: full_action_width,
            status_segments: segment_widths.len(),
        };
    }
    // The routine status detail sheds before the action detail truncates:
    // keep the longest leading prefix that still leaves the untruncated
    // action room beside the notice.
    let mut kept = 0;
    let mut used = lead_width;
    for (index, width) in segment_widths.iter().enumerate() {
        if used + full_action_width + *width > base {
            break;
        }
        used += *width;
        kept = index + 1;
    }
    if used + full_action_width <= base {
        return BusyRowPlan {
            lead_painted: lead_width > 0,
            action_room: full_action_width,
            status_segments: kept,
        };
    }
    // The action detail truncates, down to the bare tool name; the notice
    // sheds only when even the bare action cannot fit beside it. The hint
    // never sheds.
    let lead_painted = lead_width > 0 && lead_width + bare_action_width <= base;
    let lead = if lead_painted { lead_width } else { 0 };
    BusyRowPlan {
        lead_painted,
        action_room: base.saturating_sub(lead),
        status_segments: 0,
    }
}

/// The busy line's action words: `thinking` while no tool runs, otherwise
/// `running {tool}` plus the call's target through the shared detail seam —
/// the path for a write, the program for a command, the SQL for a query.
/// Names the action and its target, never a motive. `room` is the char
/// budget for the whole `running … ` phrase: the caller passes what the
/// plan affords beside the painted tail, and only the detail truncates —
/// the tool name, the elapsed time, and the cancel hint never move.
pub(super) fn action_text(
    activity: Option<&str>,
    running: Option<(String, serde_json::Value)>,
    room: usize,
) -> String {
    let bare = bare_text(activity);
    let Some(tool) = activity.map(str::to_string) else {
        return bare;
    };
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
        + Span::styled(SEPARATOR, bar).width()
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
    frame_width + action_text(activity, running, usize::MAX).chars().count() + elapsed_width + tail
}
