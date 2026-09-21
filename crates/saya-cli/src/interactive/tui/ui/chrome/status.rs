//! Status bar rendering.

use super::super::surface::SPINNER;
use super::super::theme::{accent, danger, on_accent, secondary, success, warning};
use super::action_line::{
    CANCEL_HINT, SEPARATOR, action_text, bare_action_width, busy_row_plan, running_call,
};
use crate::interactive::session_prompt::StatusView;
use crate::interactive::tui::types::App;
use crate::interactive::tui::wrap::cell_width;
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
pub(super) fn approval_colour(mode: &str) -> Color {
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
/// `crate::interactive::tui::ui::chrome::status::status_words_for_test`.
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
pub(super) fn status_spans(view: &StatusView, bg: Color) -> Vec<Span<'static>> {
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

/// The quiet "new activity" affordance: how many rows landed below a
/// scrolled-up reader, with the key that returns. `None` at the tail, where
/// the count is zero by construction — the bar stays exactly as it was while
/// following, and a scrolled reader never has their viewport taken.
fn unseen_span(count: usize, bg: Color) -> Option<Span<'static>> {
    if count == 0 {
        return None;
    }
    let noun = if count == 1 { "line" } else { "lines" };
    Some(Span::styled(
        format!("{count} new {noun} below · Shift+End to catch up "),
        Style::default().bg(bg).fg(accent()),
    ))
}

/// Renders the status bar as a filled accent-tinted strip, with a spinner and
/// hint while an agent request is streaming.
pub(in crate::interactive::tui) fn draw_status(
    frame: &mut Frame<'_>,
    app: &App,
    status: &StatusView,
    area: Rect,
) {
    let bar = Style::default()
        .bg(super::super::theme::status_bg())
        .fg(secondary());
    let bg = super::super::theme::status_bg();
    let unseen = unseen_span(app.unseen_new_rows(), bg);
    let line = if app.is_busy() {
        let frame_char = SPINNER[app.spinner % SPINNER.len()];
        let elapsed = app
            .request
            .started
            .map(|start| start.elapsed().as_secs())
            .unwrap_or(0);
        // The stop affordance is reserved first: the cancel hint and the
        // fixed chrome it rides with come off the frame width before
        // anything elastic is sized, and what is left is spent down the
        // shedding hierarchy — the routine status detail sheds, then the
        // action detail truncates, and the notice yields last — so the hint
        // is never what the row drops (audit F06: it painted last and paid
        // for every other span's overflow).
        let segments = status_spans(status, bg);
        let widths: Vec<usize> = segments.iter().map(|span| span.width()).collect();
        let unseen_width = unseen.as_ref().map(|span| span.width()).unwrap_or(0);
        let elapsed_width = cell_width(&format!("{elapsed}s "));
        let frame_width = cell_width(&format!(" {frame_char} "));
        let full_action = action_text(
            app.request.activity.as_deref(),
            running_call(app),
            usize::MAX,
        );
        let plan = busy_row_plan(
            area.width as usize,
            unseen_width,
            frame_width,
            elapsed_width,
            &widths,
            cell_width(&full_action),
            bare_action_width(app.request.activity.as_deref()),
        );
        let doing = action_text(
            app.request.activity.as_deref(),
            running_call(app),
            plan.action_room,
        );
        // The new-activity count leads the bar: it is an invitation, never a
        // jump, and the plan sheds it only when the bare action cannot fit
        // beside it. It is painted before the tail, so it is never clipped
        // by what follows.
        let mut spans = Vec::new();
        if plan.lead_painted
            && let Some(span) = unseen
        {
            spans.push(span);
        }
        spans.push(Span::styled(
            format!(" {frame_char} {doing}{elapsed}s "),
            Style::default().bg(bg).fg(accent()),
        ));
        spans.push(Span::styled(SEPARATOR, bar));
        spans.extend(segments.into_iter().take(plan.status_segments));
        spans.push(Span::styled(CANCEL_HINT, bar));
        Line::from(spans)
    } else if app.overlays.selection_mode {
        let mut spans = Vec::new();
        if let Some(span) = unseen {
            spans.push(span);
        }
        spans.push(Span::styled(
            " SELECT ",
            Style::default()
                .bg(accent())
                .fg(on_accent())
                .add_modifier(Modifier::BOLD),
        ));
        spans.extend(status_spans(status, bg));
        spans.push(Span::styled(
            "  ·  drag to copy  ·  Ctrl+O to resume scrolling ",
            bar,
        ));
        Line::from(spans)
    } else {
        let mut spans = Vec::new();
        if let Some(span) = unseen {
            spans.push(span);
        }
        spans.extend(status_spans(status, bg));
        spans.push(Span::styled("  ·  ? for help ", bar));
        Line::from(spans)
    };
    frame.render_widget(Paragraph::new(line).style(bar), area);
}

#[cfg(test)]
#[path = "status_cancel_tests.rs"]
mod cancel_tests;
#[cfg(test)]
#[path = "status_cell_tests.rs"]
mod status_cell_tests;
#[cfg(test)]
#[path = "status_tests.rs"]
mod tests;
#[cfg(test)]
pub(super) use super::super::theme::status_bg;
