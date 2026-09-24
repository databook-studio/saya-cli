//! Status bar rendering.

use super::super::surface::SPINNER;
use super::super::theme::{accent, on_accent, secondary};
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
use status_segments::{BarWords, bar_spans, bar_words, fit_bar, full_fit, segments_width};

#[path = "status_segments.rs"]
mod status_segments;

/// The approval mode's colour, one explicit arm per mode the grammar parses
/// — read-only green, ask amber, never and bypass red. Painted on the top
/// context line's `Approval:` segment (the bottom bar no longer names the
/// mode). The catch-all is a named hole, not a licence: a mode added to
/// `FromStr` but not here would render grey with nothing failing, which is
/// exactly what the colour-map test pins.
pub(super) fn approval_colour(mode: &str) -> Color {
    match mode {
        "read-only" => super::super::theme::success(),
        "ask" => super::super::theme::warning(),
        "never" => super::super::theme::danger(),
        "bypass" => super::super::theme::danger(),
        _ => secondary(),
    }
}

/// The status bar's plain-text words, in bar order — the seam the headless
/// parity test reads. Reachable as
/// `crate::interactive::tui::ui::chrome::status::status_words_for_test`.
#[cfg(test)]
pub(crate) fn status_words_for_test(view: &StatusView) -> String {
    let words = bar_words(view);
    bar_spans(&words, full_fit(&words), Color::Reset)
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

/// The idle/selection bar's plain-text words at `width` cells — segments,
/// then the right-aligned hint — the seam `status_split_tests` reads,
/// through the same [`push_bar_row`] the real row paints with. Words, never
/// spans: colour is decoration only, mirroring `context_words_for_test`.
#[cfg(test)]
pub(crate) fn bar_words_for_test(view: &StatusView, hint: &'static str, width: u16) -> String {
    let words = bar_words(view);
    let mut spans = Vec::new();
    push_bar_row(
        &mut spans,
        &words,
        hint,
        width as usize,
        Color::Reset,
        Style::default(),
    );
    spans.iter().map(|span| span.content.as_ref()).collect()
}

/// Fits the bar's segments to `width` cells, appends them, then — when the
/// hint still fits — a padding span and the hint, so its last cell lands on
/// the row's last column.
fn push_bar_row(
    spans: &mut Vec<Span<'static>>,
    words: &BarWords,
    hint: &'static str,
    width: usize,
    bg: Color,
    bar: Style,
) {
    let keep = fit_bar(words, cell_width(hint), width);
    spans.extend(bar_spans(words, keep, bg));
    if keep.hint {
        let pad = width.saturating_sub(segments_width(words, keep) + cell_width(hint));
        spans.push(Span::styled(" ".repeat(pad), bar));
        spans.push(Span::styled(hint, bar));
    }
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
    let words = bar_words(status);
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
        let segments = bar_spans(&words, full_fit(&words), bg);
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
        // The cancel hint rides at the row's end whatever room is left: when
        // there is spare width, a padding span pushes it to the last column;
        // when there is none, it paints immediately, exactly as before.
        let used = spans.iter().map(Span::width).sum::<usize>();
        let hint_width = cell_width(CANCEL_HINT);
        if used + hint_width < area.width as usize {
            let pad = area.width as usize - used - hint_width;
            spans.push(Span::styled(" ".repeat(pad), bar));
        }
        spans.push(Span::styled(CANCEL_HINT, bar));
        Line::from(spans)
    } else if app.overlays.selection_mode {
        let mut spans = Vec::new();
        let mut lead_width = 0usize;
        if let Some(span) = unseen {
            lead_width += span.width();
            spans.push(span);
        }
        let badge = Span::styled(
            " SELECT ",
            Style::default()
                .bg(accent())
                .fg(on_accent())
                .add_modifier(Modifier::BOLD),
        );
        lead_width += badge.width();
        spans.push(badge);
        push_bar_row(
            &mut spans,
            &words,
            "drag to copy · Ctrl+O to resume scrolling",
            (area.width as usize).saturating_sub(lead_width),
            bg,
            bar,
        );
        Line::from(spans)
    } else {
        let mut spans = Vec::new();
        let mut lead_width = 0usize;
        if let Some(span) = unseen {
            lead_width = span.width();
            spans.push(span);
        }
        push_bar_row(
            &mut spans,
            &words,
            "? for help",
            (area.width as usize).saturating_sub(lead_width),
            bg,
            bar,
        );
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
#[path = "status_split_tests.rs"]
mod status_split_tests;
#[cfg(test)]
#[path = "status_tests.rs"]
mod tests;
