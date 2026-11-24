//! The activity group of a turn: the step lines `02` §3.2 keeps on screen —
//! one compact line per call, naming the command, path or query it ran — and
//! the one line the group folds to when the operator folds it.
//!
//! The group is a summary header over its steps. In the compact layout the last
//! few steps are shown with their own words cut to the width, and the older ones
//! are counted; under `Ctrl+O` every step shows its whole command and the
//! outcome it reported, wrapped rather than cut. A failed block never folds:
//! what went wrong is the part the operator has to read. Nothing here names a
//! role label: the group's header is its glyph and its words.

use std::path::Path;
use std::time::Duration;

use ratatui::text::{Line, Span};

use super::transcript::TranscriptFacts;
use super::turn_rows::{GUTTER, STEP_INDENT, clip_words, mark_wrapped_at, marked, plain, wrapped};
use crate::front::tui::theme::{Glyph, Token, glyph, spinner, style};
use crate::front::view::{Activity, Step, StepState, elapsed_text, folded_summary, step_fact};
use crate::interactive::tui::wrap::cell_width;

/// The steps the compact layout keeps per turn; the older ones are counted
/// above them.
const COMPACT_STEPS: usize = 5;

/// The air a right-aligned hint keeps between its words and itself.
const AIR: usize = 2;

/// Where a step's outcome starts in full detail: under the step's own words.
const OUTCOME_INDENT: usize = STEP_INDENT + 2;

/// The turn's activity: its header, then its steps — as many as the layout has
/// room for, and folded to the header alone when the operator folded it.
pub(super) fn rows(
    activity: &Activity,
    elapsed: Duration,
    facts: &TranscriptFacts,
    focused: bool,
    content: usize,
) -> Vec<Line<'static>> {
    let mut out = vec![header(activity, elapsed, facts, focused, content)];
    // Full detail opens every block, folded or not: the operator asked for the
    // steps, and a key that sometimes hides them would be a key to remember
    // rather than one to press. A failed block never folds at all — what went
    // wrong is the part the operator has to read.
    if activity.folded && !activity.failed && !facts.detail {
        return out;
    }
    if facts.detail {
        for step in &activity.steps {
            out.extend(full_rows(step, facts, content));
        }
        return out;
    }
    let older = activity.steps.len().saturating_sub(COMPACT_STEPS);
    if older > 0 {
        // What the compact layout keeps off screen is counted, never hidden:
        // the operator reads how many lines the turn ran, and the last few say
        // what they were.
        let counted = format!("+{older} earlier {}", plural(older, "step", "steps"));
        out.push(plain(STEP_INDENT, &counted, Token::Dim, facts, content));
    }
    for step in activity.steps.iter().skip(older) {
        out.extend(compact_row(step, facts, content));
    }
    out
}

/// The group's header: the block's summary once the turn has ended, the running
/// turn's clock before that — and, right-aligned, the key that opens the block
/// the operator is on.
fn header(
    activity: &Activity,
    elapsed: Duration,
    facts: &TranscriptFacts,
    focused: bool,
    content: usize,
) -> Line<'static> {
    let (mark, words) = match activity.ended {
        true => (
            glyph(Glyph::Check, facts.charset),
            folded_summary(&activity.steps, elapsed),
        ),
        false => (
            glyph(Glyph::Bullet, facts.charset),
            format!("Working · {}", elapsed_text(elapsed)),
        ),
    };
    let hint = hint(activity, focused, facts);
    let room = content.saturating_sub(cell_width(mark) + 1 + cell_width(&hint) + AIR);
    let words = clip_words(&words, room);
    let used = cell_width(mark) + 1 + cell_width(&words) + cell_width(&hint);
    let air = content.saturating_sub(used).max(AIR);
    Line::from(vec![
        Span::raw(" ".repeat(GUTTER)),
        Span::styled(mark.to_owned(), style(Token::Ok, facts.scheme)),
        Span::raw(" "),
        Span::styled(words, style(Token::Dim, facts.scheme)),
        Span::raw(" ".repeat(air)),
        Span::styled(hint, style(Token::Dim, facts.scheme)),
    ])
}

/// The key the header names: the one that acts on the block the cursor holds,
/// and otherwise the one that opens every step in full. A hint is always
/// there, so the group never reads as a dead line.
fn hint(activity: &Activity, focused: bool, facts: &TranscriptFacts) -> String {
    if focused {
        return match activity.folded && !activity.failed {
            true => "Enter open".to_owned(),
            false => "Enter fold".to_owned(),
        };
    }
    match facts.detail {
        true => "Ctrl+O compact".to_owned(),
        false => "Ctrl+O detail".to_owned(),
    }
}

/// One step in the compact layout: its mark and the one fact that tells it
/// apart from the step above it, cut to the width at a word — and wrapped
/// rather than cut when the call failed or was denied, because the reason is
/// the part the operator has to read.
fn compact_row(step: &Step, facts: &TranscriptFacts, content: usize) -> Vec<Line<'static>> {
    let (mark, mark_token, words_token) = marks(step, facts);
    let ran = step_fact(step, facts.workspace.as_deref().map(Path::new));
    if let Some(reason) = ended_reason(step) {
        let words = format!("{ran} — {reason}");
        return mark_wrapped_at(
            STEP_INDENT,
            (mark, mark_token),
            &words,
            words_token,
            facts,
            content,
        );
    }
    let room = content.saturating_sub(STEP_INDENT + cell_width(mark) + 1);
    vec![marked(
        STEP_INDENT,
        mark,
        &clip_words(&ran, room),
        mark_token,
        words_token,
        facts,
        content,
    )]
}

/// One step in full detail: its whole command, wrapped under its mark, and the
/// outcome it reported under that.
fn full_rows(step: &Step, facts: &TranscriptFacts, content: usize) -> Vec<Line<'static>> {
    let (mark, mark_token, words_token) = marks(step, facts);
    let ran = step_fact(step, facts.workspace.as_deref().map(Path::new));
    let mut out = mark_wrapped_at(
        STEP_INDENT,
        (mark, mark_token),
        &ran,
        words_token,
        facts,
        content,
    );
    if let Some(outcome) = outcome(step) {
        out.extend(outcome_rows(outcome, facts, content));
    }
    out
}

/// The outcome a step reported, under its own step and in the dim column: the
/// words the call ended with. In full detail nothing is cut — a long outcome
/// wraps like a long command.
fn outcome_rows(outcome: &str, facts: &TranscriptFacts, content: usize) -> Vec<Line<'static>> {
    let room = content.saturating_sub(OUTCOME_INDENT).max(1);
    wrapped(outcome, room)
        .into_iter()
        .map(|row| {
            Line::from(vec![
                Span::raw(" ".repeat(OUTCOME_INDENT)),
                Span::styled(row, style(Token::Dim, facts.scheme)),
            ])
        })
        .collect()
}

/// A step's mark, and the tokens its line is painted with. A call still running
/// turns the spinner one frame per tick, so a step being waited on reads as
/// working rather than as a dot that never moves.
fn marks(step: &Step, facts: &TranscriptFacts) -> (&'static str, Token, Token) {
    match &step.state {
        StepState::Running => (spinner(facts.spinner, facts.charset), Token::Ok, Token::Fg),
        StepState::Done { .. } => (glyph(Glyph::Check, facts.charset), Token::Ok, Token::Dim),
        StepState::Failed { .. } => (glyph(Glyph::Cross, facts.charset), Token::Bad, Token::Fg),
        StepState::Denied { .. } => (glyph(Glyph::Denied, facts.charset), Token::Dim, Token::Dim),
    }
}

/// The outcome a step reported, in the step's own words: a settled summary, a
/// failure's reason or a denial's — and nothing while the call is still out.
fn outcome(step: &Step) -> Option<&str> {
    match &step.state {
        StepState::Running => None,
        StepState::Done { summary } => (!summary.is_empty()).then_some(summary.as_str()),
        StepState::Failed { summary } => Some(summary.as_str()),
        StepState::Denied { reason } => Some(reason.as_str()),
    }
}

/// The reason a compact step line carries after its own words: the part of a
/// failed or denied call that never folds away.
fn ended_reason(step: &Step) -> Option<&str> {
    match &step.state {
        StepState::Failed { summary } | StepState::Denied { reason: summary } => {
            Some(summary.as_str())
        }
        _ => None,
    }
}

/// `one` or `many` for `count`.
fn plural<'a>(count: usize, one: &'a str, many: &'a str) -> &'a str {
    match count {
        1 => one,
        _ => many,
    }
}
