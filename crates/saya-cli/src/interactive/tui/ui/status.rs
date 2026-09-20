//! Status bar rendering.

use super::panels::SPINNER;
use super::theme::{accent, danger, on_accent, secondary, status_bg, success, warning};
use crate::interactive::session_prompt::StatusView;
use crate::interactive::tui::types::App;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

/// The approval segment's colour, one explicit arm per mode the grammar
/// parses — read-only green (auto-approves reads only), ask amber (a
/// question is pending), never red (everything refuses), bypass red ("every
/// call runs without asking" is the danger it is). The catch-all is a named
/// hole, not a licence: a mode added to `FromStr` but not here would render
/// grey with nothing failing, which is exactly what the colour-map test pins.
fn approval_colour(mode: &str) -> Color {
    match mode {
        "read-only" => success(),
        "ask" => warning(),
        "never" => danger(),
        "bypass" => danger(),
        _ => secondary(),
    }
}

/// The status bar's plain-text words, in bar order — the seam the headless
/// parity test reads. The headless header's task words are a substring of
/// this line when a list is tracked, and absent from it when none is.
/// Test seam: the bar's plain-text words for the parity test. Reachable as
/// `crate::interactive::tui::ui::status_words_for_test`.
#[cfg(test)]
pub(crate) fn status_words_for_test(view: &StatusView) -> String {
    status_words(view)
}

#[cfg(test)]
fn status_words(view: &StatusView) -> String {
    let mut words = format!(
        "[{}] {}/{} approval:{} mode:{}",
        view.profile, view.provider, view.model, view.approval_mode, view.agent_mode
    );
    if let Some(summary) = view.task_summary.as_deref() {
        words.push(' ');
        words.push_str(summary);
    }
    words
}

/// Builds the coloured status-bar segments (profile, provider/model, approval,
/// mode, tasks, workspace, host, sharing), each on the bar background so they blend
/// into the strip. The `mode:` segment mirrors the headless header's, so the
/// two surfaces cannot drift.
fn status_spans(view: &StatusView, bg: Color) -> Vec<Span<'static>> {
    let base = Style::default().bg(bg);
    let approval_color = approval_colour(&view.approval_mode);
    let mut label = view.profile.clone();
    for inc in &view.included {
        label.push_str(&format!(" +{inc}"));
    }
    let mut spans = vec![
        Span::styled(
            format!(" [{label}] "),
            base.fg(accent()).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{}/{} ", view.provider, view.model),
            base.fg(secondary()),
        ),
        Span::styled(
            format!("approval:{} ", view.approval_mode),
            base.fg(approval_color),
        ),
        Span::styled(format!("mode:{} ", view.agent_mode), base.fg(secondary())),
    ];
    // The task segment names the done count while anything is tracked, and
    // is absent on an empty list — the headless header's `tasks:` shape, so
    // the two surfaces cannot drift.
    if let Some(summary) = view.task_summary.as_deref() {
        spans.push(Span::styled(format!("{summary} "), base.fg(secondary())));
    }
    // The workspace segment names the tree the session can touch, so the
    // binding is visible at every moment it matters — including on a resume
    // from a different directory.
    match view.workspace_root.as_deref() {
        Some(root) => spans.push(Span::styled(format!("ws:{root} "), base.fg(secondary()))),
        None => spans.push(Span::styled("ws:unbound ", base.fg(secondary()))),
    }
    // The host segment names the lane: unsandboxed where the host-command
    // lane composed, off where it did not, plus the denied names where the
    // session's deny list is non-empty — the same words the headless status
    // header carries, so the two surfaces cannot drift.
    let mut host = if view.host_composed {
        "host:unsandboxed".to_string()
    } else {
        "host:off".to_string()
    };
    if !view.denied_programs.is_empty() {
        host.push_str(&format!(" deny:{}", view.denied_programs.join(",")));
    }
    spans.push(Span::styled(format!("{host} "), base.fg(secondary())));
    spans.push(Span::styled(
        format!("sharing:{}", if view.sharing_on { "on" } else { "off" }),
        base.fg(if view.sharing_on {
            warning()
        } else {
            success()
        }),
    ));
    spans
}

/// The bar's cancel hint, kept verbatim in one place: the busy line's width
/// budget subtracts exactly this, so a long action detail can never push it
/// off the bar.
const CANCEL_HINT: &str = "  (Esc to cancel) ";

/// The smallest action room the bar ever grants: the full
/// `running bounded_sql_query: select region, count(*) from orders ` phrase
/// the suite pins (63 chars). A narrower terminal still renders the bar —
/// the tail overflows past the edge as today — but the action never sheds a
/// pinned target to chase a width the tail already exceeds.
const MIN_ACTION_ROOM: usize = 63;

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
fn action_text(
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
            .then(|| {
                crate::interactive::tui::stream_events::tool_call_detail_for_test(name, arguments)
            })
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
fn running_call(app: &App) -> Option<(String, serde_json::Value)> {
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
fn total_row_width(
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

/// Renders the status bar as a filled accent-tinted strip, with a spinner and
/// hint while an agent request is streaming.
pub(super) fn draw_status(frame: &mut Frame<'_>, app: &App, status: &StatusView, area: Rect) {
    let bar = Style::default().bg(status_bg()).fg(secondary());
    let bg = status_bg();
    let line = if app.is_busy() {
        let frame_char = SPINNER[app.spinner % SPINNER.len()];
        let elapsed = app
            .request
            .started
            .map(|start| start.elapsed().as_secs())
            .unwrap_or(0);
        // The bar is wider than the frame, so the room cannot come from the
        // window width minus today's tail: both already overflow the frame,
        // and subtracting them erases the target even on an idle-width bar.
        // The action sheds the row's overflow down to the frame width — but
        // never below the pinned floor (`MIN_ACTION_ROOM`), which keeps the
        // suite's target whole. A longer detail truncates to exactly what
        // fits beside the painted tail; the tail spans paint after the
        // action in the same `Line`, so the cancel hint survives a 900-char
        // detail at 100 columns by construction, and the test asserts it.
        let tail = status_spans(status, bg);
        let tail_width = tail.iter().map(|span| span.width()).sum::<usize>()
            + Span::styled(CANCEL_HINT, bar).width()
            + Span::styled("· ", bar).width();
        let elapsed_width = format!("{elapsed}s ").chars().count();
        let frame_width = format!(" {frame_char} ").chars().count();
        let full_action = action_text(
            app.request.activity.as_deref(),
            running_call(app),
            usize::MAX,
        );
        let full_row = total_row_width(frame_width, &full_action, elapsed_width, tail_width);
        // Shed the whole row overflow from the action: the tail is fixed and
        // the frame is the only width that matters. The floor keeps the
        // pinned target whole on an idle-width bar; a longer detail is what
        // pays for the overflow.
        let overflow = full_row.saturating_sub(area.width as usize);
        let room = full_action
            .chars()
            .count()
            .saturating_sub(overflow)
            .max(MIN_ACTION_ROOM.min(full_action.chars().count()));
        let doing = action_text(app.request.activity.as_deref(), running_call(app), room);
        let mut spans = vec![
            Span::styled(
                format!(" {frame_char} {doing}{elapsed}s "),
                Style::default().bg(bg).fg(accent()),
            ),
            Span::styled("· ", bar),
        ];
        spans.extend(tail);
        spans.push(Span::styled(CANCEL_HINT, bar));
        Line::from(spans)
    } else if app.overlays.selection_mode {
        let mut spans = vec![Span::styled(
            " SELECT ",
            Style::default()
                .bg(accent())
                .fg(on_accent())
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

#[cfg(test)]
#[path = "status_tests.rs"]
mod tests;
