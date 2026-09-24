//! The bottom bar's segments: database, model, mode, and tasks — split from
//! the top context line's safety posture (`context_line.rs`). Builds the
//! words, decides which sheds first when the row is too narrow, and hands
//! back styled spans so `status.rs` only wires them into the three row
//! shapes (idle, selection, busy).

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

use crate::interactive::session_prompt::StatusView;
use crate::interactive::tui::wrap::cell_width;

use super::super::super::theme::{accent, secondary};

/// The literal join between bar segments — a test-visible constant so
/// `status_split_tests` can assert on the exact words.
pub(super) const SEGMENT_SEP: &str = " · ";

/// The bar's words, in display order. The model carries no provider prefix:
/// `StatusView` already keeps `provider` and `model` apart.
pub(super) struct BarWords {
    pub name: String,
    pub plus_n: Option<String>,
    pub model: String,
    pub mode: Option<String>,
    pub tasks: Option<String>,
}

/// Builds the bar's words: the included count collapses to `+N`, and the
/// mode segment is absent at the default (`build`).
pub(super) fn bar_words(view: &StatusView) -> BarWords {
    BarWords {
        name: view.profile.clone(),
        plus_n: (!view.included.is_empty()).then(|| format!(" +{}", view.included.len())),
        model: view.model.clone(),
        mode: (view.agent_mode != "build").then(|| view.agent_mode.clone()),
        tasks: view.task_summary.clone(),
    }
}

/// Which optional pieces paint. The database's own name carries no flag —
/// it is never a candidate to drop.
#[derive(Clone, Copy)]
pub(super) struct BarFit {
    pub plus_n: bool,
    pub model: bool,
    pub mode: bool,
    pub tasks: bool,
    pub hint: bool,
}

/// Every optional piece present. Busy and selection rows keep this shape and
/// shed whole trailing segments through their own layout plans, never the
/// finer idle hierarchy below.
pub(super) fn full_fit(words: &BarWords) -> BarFit {
    BarFit {
        plus_n: words.plus_n.is_some(),
        model: true,
        mode: words.mode.is_some(),
        tasks: words.tasks.is_some(),
        hint: true,
    }
}

/// One piece's cell cost when kept: its own separator plus its text.
fn piece(keep: bool, text: Option<&str>) -> usize {
    match (keep, text) {
        (true, Some(text)) => cell_width(SEGMENT_SEP) + cell_width(text),
        _ => 0,
    }
}

/// The row's width in cells for a candidate `keep` decision: the joined
/// segments, plus a one-cell gap and the hint when it paints.
fn width_for(words: &BarWords, keep: BarFit, hint_width: usize) -> usize {
    cell_width(&name_text(words, keep.plus_n))
        + piece(keep.model, Some(&words.model))
        + piece(keep.mode, words.mode.as_deref())
        + piece(keep.tasks, words.tasks.as_deref())
        + if keep.hint { 1 + hint_width } else { 0 }
}

/// The joined segments' width alone (no hint, no gap) — callers add their
/// own gap and hint width to size the padding between the two.
pub(super) fn segments_width(words: &BarWords, mut keep: BarFit) -> usize {
    keep.hint = false;
    width_for(words, keep, 0)
}

/// Decides which pieces paint at `width` cells: sheds tasks, then mode, then
/// the `+N` count, then the model, then the hint — the database name is
/// never dropped, so the row always names what it is talking to.
pub(super) fn fit_bar(words: &BarWords, hint_width: usize, width: usize) -> BarFit {
    let mut keep = full_fit(words);
    while width_for(words, keep, hint_width) > width {
        if keep.tasks {
            keep.tasks = false;
        } else if keep.mode {
            keep.mode = false;
        } else if keep.plus_n {
            keep.plus_n = false;
        } else if keep.model {
            keep.model = false;
        } else if keep.hint {
            keep.hint = false;
        } else {
            break; // the name alone is left; let it overflow rather than vanish
        }
    }
    keep
}

/// The database name, with its `+N` suffix appended directly (no separator)
/// when `keep_plus_n` and one is present — the one piece of the name that
/// can shed on its own, ahead of the model segment.
fn name_text(words: &BarWords, keep_plus_n: bool) -> String {
    match (keep_plus_n, &words.plus_n) {
        (true, Some(suffix)) => format!("{}{suffix}", words.name),
        _ => words.name.clone(),
    }
}

/// Builds the kept segments as styled spans, in bar order: database (accent,
/// bold), model, mode, tasks (secondary) — each non-leading segment carries
/// its own [`SEGMENT_SEP`] prefix, so dropping a trailing one never leaves a
/// dangling separator.
pub(super) fn bar_spans(words: &BarWords, keep: BarFit, bg: Color) -> Vec<Span<'static>> {
    let base = Style::default().bg(bg);
    let push = |spans: &mut Vec<Span<'static>>, keep: bool, text: Option<&str>| {
        if keep && let Some(text) = text {
            spans.push(Span::styled(
                format!("{SEGMENT_SEP}{text}"),
                base.fg(secondary()),
            ));
        }
    };
    let mut spans = vec![Span::styled(
        name_text(words, keep.plus_n),
        base.fg(accent()).add_modifier(Modifier::BOLD),
    )];
    push(&mut spans, keep.model, Some(&words.model));
    push(&mut spans, keep.mode, words.mode.as_deref());
    push(&mut spans, keep.tasks, words.tasks.as_deref());
    spans
}
