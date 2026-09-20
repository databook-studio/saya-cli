//! The context line: row 0 of the TUI frame, naming what this session can
//! touch in words — the read-only database posture, the workspace binding,
//! and any unusual permission or data-sharing condition.

use super::super::theme::{secondary, warning};
use crate::interactive::session_prompt::StatusView;
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};

/// The context line's plain-text words, in row order — the seam the unit
/// tests read. Words, never spans: colour is decoration only.
#[cfg(test)]
pub(crate) fn context_words_for_test(view: &StatusView, width: u16) -> String {
    context_words(view, width as usize)
}

/// The context line's styled spans — the seam the style-blindness test
/// reads: stripping every `Style` must leave exactly `context_words`.
#[cfg(test)]
pub(crate) fn context_spans_for_test(view: &StatusView, width: u16) -> Vec<Span<'static>> {
    context_spans(view, width as usize)
}

/// How readily a segment may be dropped when the row will not fit. Lower
/// goes first. The posture is absent from this scale because it is never
/// dropped — see [`fit`].
///
/// Warnings outrank the workspace binding and the `saya` lead deliberately:
/// a narrow terminal must not be the reason "Data sharing on" or "Host
/// commands unsandboxed" stops being visible. Losing the app's own name
/// costs nothing; losing a caveat costs the user the thing this row exists
/// to tell them.
const DROP_LEAD: u8 = 0;
const DROP_WORKSPACE: u8 = 1;
const DROP_UNUSUAL: u8 = 2;

/// One segment: its words, whether it names an unusual condition (warning
/// colour), and how readily it may be dropped. Routine conditions are
/// absent, never rendered as a zero or an "off" badge.
fn segments(view: &StatusView) -> Vec<(String, bool, u8)> {
    let mut out = vec![
        ("saya".to_string(), false, DROP_LEAD),
        // The posture carries no drop rank: `fit` never removes it.
        ("Database read-only".to_string(), false, u8::MAX),
        match view.workspace_root.as_deref() {
            Some(root) => (format!("Workspace {root}"), false, DROP_WORKSPACE),
            None => ("No workspace bound".to_string(), false, DROP_WORKSPACE),
        },
    ];
    if view.sharing_on {
        out.push(("Data sharing on".to_string(), true, DROP_UNUSUAL));
    }
    if view.approval_mode != "read-only" {
        out.push((
            format!("Approval: {}", view.approval_mode),
            true,
            DROP_UNUSUAL,
        ));
    }
    if view.host_composed {
        out.push(("Host commands unsandboxed".to_string(), true, DROP_UNUSUAL));
    }
    if !view.denied_programs.is_empty() {
        out.push((
            format!("Denied: {}", view.denied_programs.join(", ")),
            true,
            DROP_UNUSUAL,
        ));
    }
    out
}

/// Drops optional segments until the joined row fits `width`: the `saya`
/// lead first, then the workspace binding, then unusual conditions
/// last-stated-first. The posture is never dropped.
fn fit(mut fitted: Vec<(String, bool, u8)>, width: usize) -> Vec<(String, bool, u8)> {
    let joined_len = |fitted: &[(String, bool, u8)]| {
        fitted.iter().map(|(text, _, _)| text.len()).sum::<usize>()
            + fitted.len().saturating_sub(1) * " · ".len()
    };
    while joined_len(&fitted) > width {
        // Drop the cheapest segment: lowest rank first, and within a rank the
        // last one, so `Denied:` goes before `Data sharing on`. `u8::MAX`
        // marks the posture, which is never a candidate — when it is all that
        // is left the loop stops and the row is allowed to overflow the pane
        // rather than say nothing about what the session can touch.
        let cheapest = fitted
            .iter()
            .enumerate()
            .filter(|(_, (_, _, rank))| *rank != u8::MAX)
            .min_by_key(|(idx, (_, _, rank))| (*rank, std::cmp::Reverse(*idx)))
            .map(|(idx, _)| idx);
        match cheapest {
            Some(idx) => {
                fitted.remove(idx);
            }
            None => break,
        }
    }
    fitted
}

#[cfg(test)]
fn context_words(view: &StatusView, width: usize) -> String {
    // The non-test path reaches the same words through `context_spans`
    // (stripped of style), so this helper exists only for the words seam.
    let unstyled: String = context_spans(view, width)
        .iter()
        .map(|span| span.content.as_ref())
        .collect();
    unstyled
}

fn context_spans(view: &StatusView, width: usize) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (i, (text, unusual, _)) in fit(segments(view), width).into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ", Style::default().fg(secondary())));
        }
        let fg = if unusual { warning() } else { secondary() };
        spans.push(Span::styled(text, Style::default().fg(fg)));
    }
    spans
}

/// Renders the context line into row 0.
pub(in crate::interactive::tui) fn draw_context_line(
    frame: &mut Frame<'_>,
    status: &StatusView,
    area: Rect,
) {
    let width = area.width as usize;
    frame.render_widget(
        Paragraph::new(Line::from(context_spans(status, width))),
        area,
    );
}

#[cfg(test)]
#[path = "context_line_tests.rs"]
mod tests;
