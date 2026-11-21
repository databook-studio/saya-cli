//! One turn of the transcript: `02` §3's blocks, read from the view model's
//! entries and laid out as rows.
//!
//! The operator's message is a quoted block, the activity group shows its live
//! steps while a turn runs and folds to one summary line when it settles, the
//! answer is markdown, and the SQL line and the result's preview sit
//! under the block the answer rests on. No block carries a role label: `02` §3
//! takes `YOU`, `SAYA` and `ACTIVITY` away.
//!
//! What one block paints is its own module's business — [`super::turn_activity`]
//! the group, [`super::result`] the query and its result,
//! [`super::turn_rows`] the cells every block shares.

use std::time::Duration;

use ratatui::text::{Line, Span};

use super::transcript::TranscriptFacts;
use super::turn_rows::{GUTTER, indented, wrapped};
use super::{blocks, blocks_question, blocks_stop, result, turn_activity};
use crate::front::tui::theme::{Glyph, Token, glyph, style};
use crate::front::view::{Entry, TurnView};
use crate::interactive::tui::markdown_spans_fenced;
use crate::interactive::tui::wrap::cell_width;

/// One turn's rows, its blocks in reading order.
pub(super) fn rows(
    turn: &TurnView,
    elapsed: Duration,
    facts: &TranscriptFacts,
    width: usize,
) -> Vec<Line<'static>> {
    let content = width.saturating_sub(2 * GUTTER).max(1);
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut above: Option<&Entry> = None;
    for entry in turn.entries() {
        let painted = block(entry, elapsed, facts, content);
        if painted.is_empty() {
            continue;
        }
        let touched = match above {
            Some(above) => sits_directly_under(above, entry),
            None => true,
        };
        if !touched {
            // One blank row between blocks: `02` §2 spaces its blocks apart.
            out.push(Line::default());
        }
        out.extend(painted);
        above = Some(entry);
    }
    out
}

/// Whether `below` sits directly under `above`, with no blank row between: a
/// folded activity block's one line is the header of the answer it summarizes
/// (`02` §2 draws `✓ 4 steps …` straight above the answer).
fn sits_directly_under(above: &Entry, below: &Entry) -> bool {
    match (above, below) {
        (Entry::Activity(activity), Entry::Answer(_)) => activity.folded && !activity.failed,
        _ => false,
    }
}

/// One block of the turn, as the rows it paints.
fn block(
    entry: &Entry,
    elapsed: Duration,
    facts: &TranscriptFacts,
    content: usize,
) -> Vec<Line<'static>> {
    match entry {
        Entry::UserMessage(message) => quoted(&message.text, facts, content),
        Entry::Activity(activity) => turn_activity::rows(activity, elapsed, facts, content),
        Entry::Answer(answer) => answer_rows(&answer.text, content),
        Entry::ResultPreview(preview) => result::rows(preview, facts, content),
        Entry::Notice(notice) => blocks::system_line(notice, facts, content),
        Entry::Edit(edit) => blocks::edit_rows(edit, facts, content),
        Entry::Command(command) => blocks::command_rows(command, facts, content),
        Entry::Helper(helper) => blocks::helper_rows(helper, facts, content),
        Entry::Error(error) => blocks_stop::error_rows(error, facts, content),
        Entry::Stop(stop) => blocks_stop::stop_rows(stop, facts, content),
        Entry::Question(question) => blocks_question::question_rows(question, facts, content),
    }
}

/// The operator's own words: `❯` and the message, with no label above them.
fn quoted(text: &str, facts: &TranscriptFacts, content: usize) -> Vec<Line<'static>> {
    let mark = format!("{} ", glyph(Glyph::Prompt, facts.charset));
    let room = content.saturating_sub(cell_width(&mark)).max(1);
    wrapped(text, room)
        .into_iter()
        .enumerate()
        .map(|(index, row)| {
            let prefix = match index {
                0 => mark.clone(),
                _ => " ".repeat(cell_width(&mark)),
            };
            Line::from(vec![
                Span::raw(" ".repeat(GUTTER)),
                Span::styled(prefix, style(Token::User, facts.scheme)),
                Span::styled(row, style(Token::User, facts.scheme)),
            ])
        })
        .collect()
}

/// Saya's answer: today's markdown renderer, one row per wrapped line.
///
/// The renderer is the interactive surface's own ([`markdown_spans_fenced`]),
/// reused rather than copied, so a marker the model writes — bold, inline code,
/// a fence, a bullet — reads the same in both surfaces. The wrap happens first,
/// in cells, so a row can never be wider than the pane whatever the renderer
/// hands back.
fn answer_rows(text: &str, content: usize) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let mut fence = false;
    for line in text.lines() {
        let rows = match !fence && opens_a_bullet(line) {
            true => bullet(line, content),
            false => wrapped(line, content),
        };
        for row in rows {
            out.push(indented(markdown_spans_fenced(&row, &mut fence)));
        }
    }
    out
}

/// Whether a markdown line opens a bullet item.
fn opens_a_bullet(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("- ") || trimmed.starts_with("* ")
}

/// A bullet item's rows: the marker on the first, the marker's width as air on
/// the rest, so a wrapped item still reads as one item.
fn bullet(line: &str, content: usize) -> Vec<String> {
    let body = line.trim_start().get(2..).unwrap_or_default();
    wrapped(body, content.saturating_sub(2).max(1))
        .into_iter()
        .enumerate()
        .map(|(index, row)| match index {
            0 => format!("- {row}"),
            _ => format!("  {row}"),
        })
        .collect()
}
