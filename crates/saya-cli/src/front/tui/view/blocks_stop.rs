//! The blocks that end a turn: the error (`02` §3.7) and what stopped it —
//! budget, pause or cancel (`02` §3.8).
//!
//! Both are pure painters, and neither ever folds: an error's reason and a
//! stop's sentence are the part the operator has to read, so they wrap instead
//! of being cut. The budget's second row is `budget_partial_message` verbatim —
//! the one wording the goldens and scripts read.
//!
//! The operator's cancel is the third stop, and the only one that is not a
//! failure: the turn ends, the run stays open.

use ratatui::text::{Line, Span};

use super::transcript::TranscriptFacts;
use super::turn_rows::{STEP_INDENT, marked_wrapped, plain, wrapped};
use crate::budget_wording::budget_partial_message;
use crate::front::tui::theme::{Glyph, Token, glyph, style};
use crate::front::view::{Error, Stop};

/// What the operator can still do after a turn failed (`02` §3.7): the message
/// is kept, so nothing said has to be typed again.
const KEPT: &str = "Your message is kept: ↑ to edit, Enter to resend.";

/// The budget stop's own name (`02` §3.8): the database plugin armed it when the
/// turn first touched the database.
const BUDGET_TITLE: &str = "Database budget reached";

/// The budget stop's way on, before the limit that bound it is named.
const BUDGET_ONWARD: &str = "Ask again to continue";

/// The pause's way on (`02` §3.8).
const PAUSED_ONWARD: &str = "Enter continue · type to redirect · /status for details";

/// The cancel's one sentence (`02` §3.8).
const CANCELLED: &str = "Stopped by you. Nothing after this point ran.";

/// An error: its reason, then what the operator can still do (`02` §3.7).
pub(super) fn error_rows(
    error: &Error,
    facts: &TranscriptFacts,
    content: usize,
) -> Vec<Line<'static>> {
    let mut out = marked_wrapped(
        (glyph(Glyph::Cross, facts.charset), Token::Bad),
        &error.message,
        Token::Fg,
        facts,
        content,
    );
    out.push(plain(STEP_INDENT, KEPT, Token::Dim, facts, content));
    out
}

/// What stopped the turn: the budget's three rows, the pause's two, or the
/// cancel's one (`02` §3.8).
pub(super) fn stop_rows(
    stop: &Stop,
    facts: &TranscriptFacts,
    content: usize,
) -> Vec<Line<'static>> {
    match stop {
        Stop::Budget { dimension, limit } => {
            let mut out = marked_wrapped(
                (glyph(Glyph::Stop, facts.charset), Token::Warn),
                BUDGET_TITLE,
                Token::Fg,
                facts,
                content,
            );
            out.extend(sentence(
                &budget_partial_message(*dimension),
                facts,
                content,
            ));
            let onward = match limit {
                Some(limit) => format!("{BUDGET_ONWARD} · limit: {limit}"),
                None => BUDGET_ONWARD.to_owned(),
            };
            out.push(plain(STEP_INDENT, &onward, Token::Dim, facts, content));
            out
        }
        Stop::Paused { message } => {
            let mut out = marked_wrapped(
                (glyph(Glyph::Paused, facts.charset), Token::Warn),
                &format!("Paused — {message}"),
                Token::Fg,
                facts,
                content,
            );
            out.push(plain(
                STEP_INDENT,
                PAUSED_ONWARD,
                Token::Dim,
                facts,
                content,
            ));
            out
        }
        Stop::Cancelled => marked_wrapped(
            (glyph(Glyph::Denied, facts.charset), Token::Dim),
            CANCELLED,
            Token::Dim,
            facts,
            content,
        ),
    }
}

/// A stop's own sentence under its name: wrapped, never cut — the budget's
/// wording is read by the goldens and scripts, so every word of it stays.
fn sentence(text: &str, facts: &TranscriptFacts, content: usize) -> Vec<Line<'static>> {
    let room = content.saturating_sub(STEP_INDENT).max(1);
    wrapped(text, room)
        .into_iter()
        .map(|row| {
            Line::from(vec![
                Span::raw(" ".repeat(STEP_INDENT)),
                Span::styled(row, style(Token::Fg, facts.scheme)),
            ])
        })
        .collect()
}
