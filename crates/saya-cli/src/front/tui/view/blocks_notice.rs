//! The session's own line (`02` §3.10): one dim row per logical line, and as
//! many rows as a line needs. A notice is the session's own sentence, so a
//! terminal narrower than the sentence wraps it rather than cutting it.

use ratatui::text::Line;

use super::transcript::TranscriptFacts;
use super::turn_rows::{GUTTER, plain, wrapped};
use crate::front::tui::theme::Token;
use crate::front::view::Notice;

/// Something the session itself has to say: dim lines (`02` §3.10).
///
/// One line is the shape nearly every notice takes. A notice that carries
/// lines of its own — `/help`'s listing, which the chat layer's read answers
/// with — keeps them: every line is a row of its own, so the transcript shows
/// the listing as it was written rather than as one run-on sentence.
///
/// A line longer than the row it is painted in takes another row rather than an
/// ellipsis. A notice is the session's own sentence, and a terminal narrower
/// than the sentence must still be able to read all of it: an operator's
/// 50-column window used to cut the sentence mid-word.
pub(super) fn system_line(
    notice: &Notice,
    facts: &TranscriptFacts,
    content: usize,
) -> Vec<Line<'static>> {
    let room = content.saturating_sub(GUTTER).max(1);
    notice
        .message
        .lines()
        .flat_map(|line| wrapped(line, room))
        .map(|row| plain(GUTTER, &row, Token::Dim, facts, content))
        .collect()
}
