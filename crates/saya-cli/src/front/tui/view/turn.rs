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
use super::{
    blocks, blocks_question, blocks_stop, highlight, inspector, result, turn_activity, turn_focus,
};
use crate::front::tui::focus::EntryPath;
use crate::front::tui::theme::{Charset, Glyph, Token, glyph, style};
use crate::front::view::{Entry, TurnView};
use crate::interactive::tui::markdown_spans;
use crate::interactive::tui::wrap::cell_width;

/// One turn's rows, its blocks in reading order. `place` is the turn's place in
/// the conversation: the block cursor's seat names it (`02` §6).
pub(super) fn rows(
    place: usize,
    turn: &TurnView,
    elapsed: Duration,
    facts: &TranscriptFacts,
    width: usize,
) -> Vec<Line<'static>> {
    let content = width.saturating_sub(2 * GUTTER).max(1);
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut above: Option<&Entry> = None;
    let entries = turn.entries();
    let mut index = 0;
    while index < entries.len() {
        let entry = &entries[index];
        // Repeated identical notices are one sentence said once (with its
        // count), never the same row over and over.
        let mut count = 1;
        if let Entry::Notice(notice) = entry {
            while let Some(Entry::Notice(next)) = entries.get(index + count) {
                if next.message != notice.message {
                    break;
                }
                count += 1;
            }
        }
        let focused = facts.focus == Some(EntryPath::new(place, index));
        let mut painted = block(entry, elapsed, facts, focused, content);
        if painted.is_empty() {
            index += count;
            continue;
        }
        if count > 1
            && let Some(last) = painted.last_mut()
        {
            last.spans.push(Span::styled(
                count_suffix(count, facts),
                style(Token::Dim, facts.scheme),
            ));
        }
        turn_focus::mark(&mut painted, focused, facts);
        if focused
            && facts.inspector
            && let Some(line) = inspector::row(entry, facts, content)
        {
            painted.push(line);
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
        index += count;
    }
    out.extend(changed_rows(turn, facts, content));
    out
}

/// The turn's `Changed:` line (UI5): the files it changed, both counts, and the
/// key that opens their diffs.
///
/// It closes the turn rather than opening it: an operator reads what the turn
/// answered, then what it wrote. A turn that changed no file adds no row, and a
/// turn whose calls have not landed says nothing it cannot state.
fn changed_rows(turn: &TurnView, facts: &TranscriptFacts, content: usize) -> Vec<Line<'static>> {
    let Some(line) = crate::front::view::change_summary(&crate::front::view::changed(turn)) else {
        return Vec::new();
    };
    // One wording, read from the view model: the label is what the line is, and
    // the words after it are what changed — the same string the pane's title
    // names, never a second spelling of it.
    let (label, body) = match line.strip_prefix("Changed: ") {
        Some(body) => ("Changed: ", body),
        None => ("", line.as_str()),
    };
    let runs = vec![(label.to_owned(), Token::Dim), (body.to_owned(), Token::Fg)];
    vec![Line::from(
        std::iter::once(Span::raw(" ".repeat(GUTTER)))
            .chain(clipped(runs, facts, content))
            .collect::<Vec<_>>(),
    )]
}

/// The runs cut to `room` cells: the line is one row, so what does not fit is
/// left off rather than wrapped under it.
fn clipped(runs: Vec<(String, Token)>, facts: &TranscriptFacts, room: usize) -> Vec<Span<'static>> {
    let mut left = room;
    let mut out = Vec::new();
    for (text, token) in runs {
        if left == 0 {
            break;
        }
        let kept = super::palette_frame::fit(&text, left);
        left = left.saturating_sub(cell_width(&kept));
        out.push(Span::styled(kept, style(token, facts.scheme)));
    }
    out
}

/// The count a repeated notice carries: `×3`, or `x3` in the seven-bit set.
fn count_suffix(count: usize, facts: &TranscriptFacts) -> String {
    match facts.charset {
        Charset::Utf8 => format!(" ×{count}"),
        Charset::Ascii => format!(" x{count}"),
    }
}

/// Whether `below` sits directly under `above`, with no blank row between: a
/// block the operator folded to its one line is the header of the answer it
/// summarizes (`02` §2 draws `✓ 4 steps …` straight above the answer).
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
    focused: bool,
    content: usize,
) -> Vec<Line<'static>> {
    match entry {
        Entry::UserMessage(message) => quoted(&message.text, facts, content),
        Entry::Activity(activity) => {
            turn_activity::rows(activity, elapsed, facts, focused, content)
        }
        Entry::Answer(answer) => answer_rows(&answer.text, facts, content),
        Entry::ResultPreview(preview) => result::rows(preview, facts, content),
        Entry::Notice(notice) => super::blocks_notice::system_line(notice, facts, content),
        Entry::Edit(edit) => blocks::edit_rows(edit, facts, content),
        Entry::Command(command) => blocks::command_rows(command, facts, content),
        Entry::Helper(helper) => blocks::helper_rows(helper, facts, content),
        Entry::Error(error) => blocks_stop::error_rows(error, facts, content),
        Entry::Stop(stop) => blocks_stop::stop_rows(stop, facts, content),
        Entry::Question(question) => blocks_question::question_rows(question, facts, content),
    }
}

/// The operator's own words: a left bar, then `❯` and the message, with no
/// label above them. The bar is the block's distinct shape — a quote the answer
/// under it never carries — and it is drawn from the glyph set, so `--ascii`
/// gets `|` and `>` rather than a glyph the terminal cannot draw.
fn quoted(text: &str, facts: &TranscriptFacts, content: usize) -> Vec<Line<'static>> {
    let bar = glyph(Glyph::Edge, facts.charset);
    let mark = format!("{bar} {} ", glyph(Glyph::Prompt, facts.charset));
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

/// Saya's answer: the markdown renderer for its prose, and this job's reader for
/// the code a fence carries.
///
/// The prose is the interactive surface's own renderer
/// ([`markdown_spans`]), reused rather than copied, so a marker the model writes
/// — bold, inline code, a bullet — reads the same in both surfaces. The fence is
/// this pane's: it opens on a line of backticks, names its language, and its
/// body is read as code ([`highlight`]) in the active token column, so a keyword
/// is a keyword while colour is on and the same words when it is off. The wrap
/// happens first, in cells, so a row can never be wider than the pane whatever
/// the renderer hands back.
fn answer_rows(text: &str, facts: &TranscriptFacts, content: usize) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let mut fence = highlight::Fence::default();
    for line in text.lines() {
        if fence.toggle(line).is_some() {
            out.push(indented(vec![fence.marker(facts.scheme)]));
            continue;
        }
        let rows = match !fence.open() && opens_a_bullet(line) {
            true => bullet(line, content),
            false => wrapped(line, content),
        };
        for row in rows {
            let spans = match fence.open() {
                true => highlight::spans(&row, fence.language(), facts.scheme),
                false => markdown_spans(&row),
            };
            out.push(indented(spans));
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
