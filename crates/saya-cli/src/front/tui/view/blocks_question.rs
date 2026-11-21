//! The model's clarifying question (`02` §3.9): the question, the answers it
//! offered, and the cue that the answer is typed into the composer.
//!
//! A pure painter: the question's facts and the content column in, the rows
//! out. The answer is the next turn, so nothing here grants or decides
//! anything — it is the question as the transcript keeps it.

use ratatui::text::{Line, Span};

use super::transcript::TranscriptFacts;
use super::turn_rows::{STEP_INDENT, marked_wrapped, plain, wrapped};
use crate::front::tui::theme::{Glyph, Token, glyph, style};
use crate::front::view::Clarification;
use crate::interactive::tui::wrap::cell_width;

/// The cue that the answer goes into the composer, not into a key of its own
/// (`02` §3.9).
const ANSWER_CUE: &str = "Type a number or your own answer.";

/// The question, its numbered options and the cue (`02` §3.9).
pub(super) fn question_rows(
    question: &Clarification,
    facts: &TranscriptFacts,
    content: usize,
) -> Vec<Line<'static>> {
    let mut out = marked_wrapped(
        (glyph(Glyph::Ask, facts.charset), Token::Accent),
        &question.question,
        Token::Fg,
        facts,
        content,
    );
    for (index, option) in question.options.iter().enumerate() {
        out.extend(option_rows(index + 1, option, facts, content));
    }
    out.push(plain(STEP_INDENT, ANSWER_CUE, Token::Dim, facts, content));
    out
}

/// One numbered option: its number, and its words wrapped under them.
fn option_rows(
    number: usize,
    option: &str,
    facts: &TranscriptFacts,
    content: usize,
) -> Vec<Line<'static>> {
    let lead = format!("{number}. ");
    let room = content
        .saturating_sub(STEP_INDENT + cell_width(&lead))
        .max(1);
    wrapped(option, room)
        .into_iter()
        .enumerate()
        .map(|(index, row)| {
            let prefix = match index {
                0 => lead.clone(),
                _ => " ".repeat(cell_width(&lead)),
            };
            Line::from(vec![
                Span::raw(" ".repeat(STEP_INDENT)),
                Span::styled(format!("{prefix}{row}"), style(Token::Fg, facts.scheme)),
            ])
        })
        .collect()
}
