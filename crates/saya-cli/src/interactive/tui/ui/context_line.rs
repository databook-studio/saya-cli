//! The context line: row 0 of the TUI frame, naming what this session can
//! touch in words — the read-only database posture, the workspace binding,
//! and any unusual permission or data-sharing condition.

use super::theme::{secondary, warning};
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
pub(super) fn draw_context_line(frame: &mut Frame<'_>, status: &StatusView, area: Rect) {
    let width = area.width as usize;
    frame.render_widget(
        Paragraph::new(Line::from(context_spans(status, width))),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bound_routine() -> StatusView {
        StatusView {
            profile: "analytics".into(),
            included: Vec::new(),
            provider: "ollama".into(),
            model: "qwen".into(),
            approval_mode: "read-only".into(),
            agent_mode: "build".into(),
            workspace_root: Some("/home/user/proj".into()),
            sharing_on: false,
            host_composed: false,
            denied_programs: Vec::new(),
            task_summary: None,
        }
    }

    #[test]
    fn the_row_always_states_read_only_database_access() {
        for mut view in [bound_routine(), bound_routine()] {
            view.workspace_root = None;
            assert!(
                context_words_for_test(&bound_routine(), 80).contains("Database read-only"),
                "bound session states the posture"
            );
            assert!(
                context_words_for_test(&view, 80).contains("Database read-only"),
                "unbound session states the posture"
            );
        }
    }

    #[test]
    fn an_unbound_workspace_says_so_rather_than_showing_nothing() {
        let mut unbound = bound_routine();
        unbound.workspace_root = None;
        assert!(
            context_words_for_test(&unbound, 80).contains("No workspace bound"),
            "None root says so"
        );
        assert!(
            context_words_for_test(&bound_routine(), 80).contains("Workspace /home/user/proj"),
            "Some root names the binding"
        );
    }

    #[test]
    fn unusual_conditions_appear_as_words_not_colour() {
        let mut view = bound_routine();
        view.sharing_on = true;
        view.approval_mode = "bypass".into();
        view.host_composed = true;
        view.denied_programs = vec!["curl".into(), "wget".into()];
        let words = context_words_for_test(&view, 160);
        for phrase in [
            "Data sharing on",
            "Approval: bypass",
            "Host commands unsandboxed",
            "Denied: curl, wget",
        ] {
            assert!(words.contains(phrase), "row states {phrase:?} in words");
        }
        // Ignoring every Style leaves the content unchanged: the words are
        // present with or without colour, so colour is decoration only.
        let unstyled: String = context_spans_for_test(&view, 160)
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert_eq!(unstyled, words, "styles carry no content");
    }

    #[test]
    fn a_routine_session_carries_no_extra_segments() {
        let words = context_words_for_test(&bound_routine(), 80);
        for phrase in [
            "Data sharing on",
            "Approval:",
            "Host commands unsandboxed",
            "Denied:",
        ] {
            assert!(!words.contains(phrase), "routine row omits {phrase:?}");
        }
    }

    #[test]
    fn a_narrow_row_keeps_the_posture_over_optional_chrome() {
        let mut view = bound_routine();
        view.sharing_on = true;
        view.approval_mode = "bypass".into();
        view.host_composed = true;
        view.denied_programs = vec!["curl".into()];
        let words = context_words_for_test(&view, 20);
        assert!(
            words.contains("Database read-only"),
            "width 20 keeps the posture legible"
        );
        assert!(
            !words.contains("saya") && !words.contains("Workspace"),
            "width 20 drops the lead and the binding, not the posture: {words}"
        );
    }

    /// A narrow terminal must not be the reason a caveat stops being visible.
    /// At width 40 there is room for the posture and one warning, but not for
    /// the `saya` lead and a long workspace path as well — so those go first.
    /// The earlier ranking dropped all four warnings and kept the app's own
    /// name, which is exactly backwards for the row that exists to say what
    /// the session can touch.
    #[test]
    fn a_warning_outranks_the_lead_and_the_workspace_path() {
        let mut view = bound_routine();
        view.sharing_on = true;
        let words = context_words_for_test(&view, 40);
        assert!(
            words.contains("Data sharing on"),
            "a warning survives a narrow row while chrome is dropped: {words}"
        );
        assert!(
            words.contains("Database read-only"),
            "the posture survives alongside it: {words}"
        );
        assert!(
            !words.contains("saya"),
            "the lead is dropped first — it is pure chrome: {words}"
        );
    }

    /// Within the warnings themselves the last stated goes first, so the
    /// ordering is stable and `Data sharing on` outlives `Denied:`.
    #[test]
    fn warnings_are_dropped_last_stated_first() {
        let mut view = bound_routine();
        view.sharing_on = true;
        view.denied_programs = vec!["curl".into()];
        let words = context_words_for_test(&view, 45);
        assert!(
            words.contains("Data sharing on") && !words.contains("Denied"),
            "the later warning is shed before the earlier one: {words}"
        );
    }
}
