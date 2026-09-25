//! Chapters: one request and everything it produced, held together.
//!
//! Phase 3, packet 2 adds the manual fold on top of packet 1's numbering:
//! a finished chapter collapses to one row carrying its verbatim request
//! text, and the same keystroke reopens it. Fold state is view state — the
//! set lives on [`Transcript`](super::Transcript), never persisted, never
//! replayed — following the `Block.group` precedent.
//!
//! Blocks pushed before any `User` block exists (the welcome message, a
//! resumed-session divider) carry chapter `0` (`PRE_CHAPTER`): a plain
//! integer every read site can compare and group by without unwrapping,
//! sorting before all real chapters.
//!
//! [`Block`]: super::Block
//! [`BlockKind::User`]: super::BlockKind::User

use super::view::rows::{Row, WrappedLines, label, wrap_word_aware};
use super::{Block, BlockKind, Transcript};
use crate::interactive::tui::wrap::{cell_width, truncate_cells};

/// Chapter of blocks pushed before any request: no `User` block yet, so no
/// chapter has begun. Plain `0` (not `Option`) so read sites compare and
/// group without unwrapping; it sorts before all real chapters.
pub(crate) const PRE_CHAPTER: u32 = 0;

/// The chapter a new block of `kind` joins: a `User` block opens the chapter
/// after the current one, anything else joins the current chapter. Derives
/// from the surviving tail, never a recount, so a partly evicted chapter
/// keeps its number and the next request still advances past it.
pub(crate) fn chapter_for(blocks: &[Block], kind: BlockKind) -> u32 {
    let current = current_chapter(blocks);
    if kind == BlockKind::User {
        current + 1
    } else {
        current
    }
}

/// The newest chapter with a surviving block: the last block's chapter, or
/// `PRE_CHAPTER` when the transcript is empty.
pub(crate) fn current_chapter(blocks: &[Block]) -> u32 {
    blocks.last().map(|b| b.chapter).unwrap_or(PRE_CHAPTER)
}

/// The half-open block range carrying `chapter`: `None` when no surviving
/// block does (an evicted chapter, or one never begun). Contiguous by
/// construction — chapters only advance — so first-to-last match is the
/// whole run, never a panic on a partly evicted chapter.
#[allow(dead_code)]
pub(crate) fn chapter_range(blocks: &[Block], chapter: u32) -> Option<(usize, usize)> {
    let start = blocks.iter().position(|b| b.chapter == chapter)?;
    let end = blocks
        .iter()
        .rposition(|b| b.chapter == chapter)
        .map(|i| i + 1)
        .unwrap_or(start);
    Some((start, end))
}

/// The verbatim request opening `chapter`: the first surviving `User` block
/// of that chapter, flattened to one line. `None` when the opener was
/// evicted (mid-chapter survivors only) or the chapter never began — the
/// fold then has nothing honest to show, so it refuses.
pub(crate) fn chapter_request(blocks: &[Block], chapter: u32) -> Option<String> {
    let block = blocks
        .iter()
        .find(|b| b.chapter == chapter && b.kind == BlockKind::User)?;
    Some(request_line(&block.text))
}

/// Whether `chapter` may fold: a finished real chapter with its request
/// text intact. The live chapter never folds (work in progress must stay
/// visible); `PRE_CHAPTER` is not a request; an evicted opener refuses.
pub(crate) fn foldable(blocks: &[Block], chapter: u32) -> bool {
    chapter != PRE_CHAPTER
        && chapter != current_chapter(blocks)
        && chapter_request(blocks, chapter).is_some()
}

/// The newest foldable chapter, if any: chapters only advance, so the
/// search walks back from the chapter before the live one.
pub(crate) fn latest_foldable(blocks: &[Block]) -> Option<u32> {
    let current = current_chapter(blocks);
    (1..current)
        .rev()
        .find(|&chapter| foldable(blocks, chapter))
}

/// Rows the open chapter would paint, counted exactly the way `lines()`
/// emits them (label row plus wrapped body rows; empty blocks stay one
/// bare row). Shared by the folded summary's hidden-line count so the
/// count names the rows the fold hides — never an estimate.
pub(crate) fn unfolded_row_count(block: &Block, width: usize) -> usize {
    if block.text.is_empty() {
        return 1;
    }
    let mut rows = if Row::label(block.kind).is_some() {
        1
    } else {
        0
    };
    for raw in block.text.split('\n') {
        rows += body_row_count(block, raw, width);
    }
    rows
}

/// One folded row: the verbatim request plus a `▸` marker naming the hidden
/// rows — the request's own words, truncated with an ellipsis when too long
/// for `width`. `start..end` is the chapter's range from the caller's one
/// sweep; this rescans nothing. `None` unless the chapter may fold — an
/// opener evicted after folding unfolds itself; a chapter become live
/// again can never paint folded.
pub(crate) fn folded_row(
    blocks: &[Block],
    chapter: u32,
    start: usize,
    end: usize,
    width: usize,
) -> Option<Row> {
    if chapter == PRE_CHAPTER || chapter == current_chapter(blocks) {
        return None;
    }
    // The opener is the chapter's first surviving `User` block, inside its
    // run by contiguity.
    let run = &blocks[start..end];
    let opener = run.iter().find(|b| b.kind == BlockKind::User)?;
    let request = request_line(&opener.text);
    let eff = width.max(1);
    let shown: usize = run.iter().map(|b| unfolded_row_count(b, eff)).sum();
    let marker = format!(" ▸ {} lines", shown.saturating_sub(1).max(1));
    // The folded row still says whose turn it is. Without the role word it is
    // a user request painted exactly like body prose, so after the preceding
    // chapter it reads as that chapter's continuation — the misattribution
    // Phase 2 fixed for `System` content. The word rides in the text rather
    // than making this a label row, because find skips label rows and a
    // folded request must stay findable.
    let lead = label(BlockKind::User).unwrap_or("YOU");
    let prefix = format!("{lead}  ");
    let room = eff.saturating_sub(cell_width(&marker) + cell_width(&prefix));
    let fit = room.saturating_sub(1).max(1);
    let mut text = truncate_cells(&request, fit);
    if cell_width(&request) > fit {
        text.push('…');
    }
    Some(Row::body(BlockKind::User, prefix + &text + &marker))
}

/// A block's body rows for one raw line: blank stays bare, tables stay one
/// whole row (the view clips them), everything else wraps word-aware.
fn body_row_count(block: &Block, raw: &str, width: usize) -> usize {
    if raw.is_empty() || block.kind == BlockKind::Table {
        return 1;
    }
    let mut probe: WrappedLines = Vec::new();
    wrap_word_aware(raw, width.max(1), block.kind, &mut probe);
    probe.len().max(1)
}

/// The request on one line: newlines become spaces, kept verbatim — never
/// summarised — so the folded row cannot be mistaken for a rewrite.
fn request_line(text: &str) -> String {
    text.split('\n')
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// View-state fold toggling, implemented here (on `Transcript`, next to the
/// data it reads) so `transcript.rs` keeps only the small `lines()` branch.
/// The set holds folded finished chapters; `clear()` empties it with the
/// blocks, and a chapter whose opener evicted can never render folded
/// (`folded_row` re-checks `foldable`), so stale ids unfold themselves.
impl Transcript {
    /// Whether `chapter` currently renders as one folded row.
    pub(crate) fn is_folded(&self, chapter: u32) -> bool {
        self.folded.contains(&chapter)
    }

    /// Toggles `chapter` between folded and open. Refuses (false) unless the
    /// chapter may fold — the live chapter, `PRE_CHAPTER`, an unknown
    /// chapter, and an opener-evicted chapter all stay open.
    pub(crate) fn toggle_chapter(&mut self, chapter: u32) -> bool {
        if self.is_folded(chapter) {
            self.folded.remove(&chapter);
            self.invalidate_cache();
            return true;
        }
        if !foldable(&self.blocks, chapter) {
            return false;
        }
        self.folded.insert(chapter);
        self.invalidate_cache();
        true
    }

    /// Toggles the newest foldable chapter (the one the user just finished),
    /// for Enter on an empty line. False when nothing may fold.
    pub(crate) fn toggle_latest_chapter(&mut self) -> bool {
        let next = latest_foldable(&self.blocks);
        next.is_some_and(|chapter| self.toggle_chapter(chapter))
    }

    /// Folds the chapter that just finished at the live edge, for a new
    /// request starting. Inserts only — never toggles, never unfolds — so a
    /// chapter the user deliberately reopened stays open. Guards: fold only
    /// while following the tail (no text-selection state exists, so scrolled
    /// away is the honest proxy for "reading something that folding would
    /// hide"), and only over a real chapter with its request intact — never
    /// `PRE_CHAPTER`, never an already-folded chapter.
    ///
    /// A material caveat is NOT guarded: no block, field, or type marks a
    /// caveat anywhere, and no heuristic sniffs text for warning words — a
    /// guard that claimed to cover caveats would be a lie. Only the live-edge
    /// caller (`App::submit`, idle path) may call this: folding on a timer,
    /// on scroll, on completion, or on session resume is out.
    pub(crate) fn auto_fold_finished_chapter(&mut self) -> bool {
        if !self.is_following_tail() {
            return false;
        }
        // Fold the chapter the new request finishes — the live chapter now,
        // the previous one once the `User` block lands. `foldable` answers
        // over the pre-push numbering, so check the request survives here and
        // insert this same id after `push`: an already-folded chapter is left
        // alone (insert-only, never toggle), and a chapter the user reopened
        // stays open.
        let chapter = current_chapter(&self.blocks);
        if chapter == PRE_CHAPTER
            || self.is_folded(chapter)
            || chapter_request(&self.blocks, chapter).is_none()
        {
            return false;
        }
        self.folded.insert(chapter);
        self.invalidate_cache();
        true
    }
}

#[cfg(test)]
#[path = "chapters_tests.rs"]
mod chapter_tests;

#[cfg(test)]
#[path = "autofold_tests.rs"]
mod autofold_tests;
#[cfg(test)]
#[path = "fold_cell_tests.rs"]
mod fold_cell_tests;
