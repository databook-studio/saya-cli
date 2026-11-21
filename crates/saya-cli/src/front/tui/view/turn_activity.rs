//! The activity group of a turn: the steps `02` §3.2 keeps visible while the
//! turn runs, and the one line it folds to when it settles.
//!
//! A failed block never folds — what went wrong is the part the operator has to
//! read. Nothing here names a role label: the group's header is its glyph and
//! its words.

use std::path::Path;
use std::time::Duration;

use ratatui::text::{Line, Span};

use super::transcript::TranscriptFacts;
use super::turn_rows::{GUTTER, STEP_INDENT, marked};
use crate::front::tui::theme::{Glyph, Token, glyph, style};
use crate::front::view::{Activity, Step, StepState, elapsed_text, folded_summary, step_line};
use crate::interactive::tui::wrap::{cell_width, truncate_cells};

/// The steps `02` §3.2 keeps visible while the turn runs; older ones counted.
const LIVE_STEPS: usize = 4;

/// The air a right-aligned fold hint keeps between its words and itself.
const AIR: usize = 2;

/// The turn's activity: the live group while it runs, its one summary line once
/// it settles.
pub(super) fn rows(
    activity: &Activity,
    elapsed: Duration,
    facts: &TranscriptFacts,
    content: usize,
) -> Vec<Line<'static>> {
    if activity.folded && !activity.failed {
        let hint = format!("{} Enter", glyph(Glyph::Selected, facts.charset));
        let line = format!(
            "{} {}",
            glyph(Glyph::Check, facts.charset),
            folded_summary(&activity.steps, elapsed)
        );
        return vec![headed(&line, Some(&hint), facts, content)];
    }
    live(activity, elapsed, facts, content)
}

/// The live group: its header, then the steps `02` §3.2 keeps visible. Older
/// steps are counted in the header, which is also the row that carries the key
/// opening the whole group.
fn live(
    activity: &Activity,
    elapsed: Duration,
    facts: &TranscriptFacts,
    content: usize,
) -> Vec<Line<'static>> {
    let older = activity.steps.len().saturating_sub(LIVE_STEPS);
    let bullet = glyph(Glyph::Bullet, facts.charset);
    let time = elapsed_text(elapsed);
    let header = match older {
        0 => format!("{bullet} Working · {time}"),
        older => format!("{bullet} Working · {time} · {older} earlier steps"),
    };
    let mut out = Vec::new();
    match older {
        0 => out.push(headed(&header, None, facts, content)),
        _ => {
            let hint = format!("{} Enter", glyph(Glyph::Selected, facts.charset));
            out.push(headed(&header, Some(&hint), facts, content));
        }
    }
    for step in activity.steps.iter().skip(older) {
        out.push(step_row(step, facts, content));
    }
    out
}

/// One tool step: its outcome glyph and the phrase `front/view/steps.rs` makes
/// of it. A call that failed or was denied keeps its own words after the
/// phrase — the line that never folds.
fn step_row(step: &Step, facts: &TranscriptFacts, content: usize) -> Line<'static> {
    let line = step_line(step, facts.workspace.as_deref().map(Path::new));
    let (mark, mark_token, words_token) = match &step.state {
        StepState::Running => (Glyph::Bullet, Token::Ok, Token::Fg),
        StepState::Done { .. } => (Glyph::Check, Token::Ok, Token::Dim),
        StepState::Failed { .. } => (Glyph::Cross, Token::Bad, Token::Fg),
        StepState::Denied { .. } => (Glyph::Denied, Token::Dim, Token::Dim),
    };
    let words = match &step.state {
        StepState::Failed { summary } | StepState::Denied { reason: summary } => {
            format!("{line} — {summary}")
        }
        _ => line,
    };
    marked(
        STEP_INDENT,
        glyph(mark, facts.charset),
        &words,
        mark_token,
        words_token,
        facts,
        content,
    )
}

/// A block's header: its glyph and words, and, when one is given, the key that
/// opens the block right-aligned against the content column's edge.
fn headed(
    left: &str,
    hint: Option<&str>,
    facts: &TranscriptFacts,
    content: usize,
) -> Line<'static> {
    let (mark, words) = left.split_once(' ').unwrap_or((left, ""));
    let hint = hint.unwrap_or_default();
    let room = content.saturating_sub(cell_width(hint) + AIR + cell_width(mark));
    let words = truncate_to(words.trim_start(), room);
    let used = cell_width(mark) + 1 + cell_width(&words) + cell_width(hint);
    // Only a header with a hint is right-aligned: one without ends with its
    // words, so it carries no air it cannot use.
    let air = match hint.is_empty() {
        true => 0,
        false => content.saturating_sub(used).max(AIR),
    };
    let mut spans = vec![
        Span::raw(" ".repeat(GUTTER)),
        Span::styled(mark.to_owned(), style(Token::Ok, facts.scheme)),
        Span::raw(" "),
        Span::styled(words, style(Token::Dim, facts.scheme)),
    ];
    if !hint.is_empty() {
        spans.push(Span::raw(" ".repeat(air)));
        spans.push(Span::styled(
            hint.to_owned(),
            style(Token::Dim, facts.scheme),
        ));
    }
    Line::from(spans)
}

/// `words` cut to `room` cells, marked with an ellipsis when it was cut.
fn truncate_to(words: &str, room: usize) -> String {
    if cell_width(words) <= room {
        return words.to_owned();
    }
    match room {
        0 => String::new(),
        room => format!("{}…", truncate_cells(words, room - 1)),
    }
}
