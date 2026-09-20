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

use super::rows::{Row, WrappedLines, label, wrap_word_aware};
use super::{Block, BlockKind};

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
#[allow(dead_code)]
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
/// rows. Never a summary or title — the request's own words, truncated with
/// an ellipsis when too long for `width` (chars, like the wrap width).
/// `None` unless the chapter may fold, so every render site honors the
/// rule: a chapter whose opener evicted after folding unfolds itself, and a
/// finished chapter that becomes live again can never paint folded.
pub(crate) fn folded_row(blocks: &[Block], chapter: u32, width: usize) -> Option<Row> {
    if !foldable(blocks, chapter) {
        return None;
    }
    let request = chapter_request(blocks, chapter)?;
    let (start, end) = chapter_range(blocks, chapter)?;
    let eff = width.max(1);
    let shown: usize = blocks[start..end]
        .iter()
        .map(|b| unfolded_row_count(b, eff))
        .sum();
    let marker = format!(" ▸ {} lines", shown.saturating_sub(1).max(1));
    // The folded row still says whose turn it is. Without the role word it is
    // a user request painted exactly like body prose, so after the preceding
    // chapter it reads as that chapter's continuation — the misattribution
    // Phase 2 fixed for `System` content. The word rides in the text rather
    // than making this a label row, because find skips label rows and a
    // folded request must stay findable.
    let lead = label(BlockKind::User).unwrap_or("YOU");
    let prefix = format!("{lead}  ");
    let room = eff.saturating_sub(marker.chars().count() + prefix.chars().count());
    let mut text: String = request
        .chars()
        .take(room.saturating_sub(1).max(1))
        .collect();
    if request.chars().count() > text.chars().count() {
        text.push('…');
    }
    text.push_str(&marker);
    Some(Row::body(BlockKind::User, format!("{prefix}{text}")))
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
impl super::Transcript {
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
}

#[cfg(test)]
mod chapter_tests {
    use super::super::{BlockKind, Transcript};
    use super::{chapter_range, current_chapter};

    #[test]
    fn a_user_block_starts_a_new_chapter() {
        let mut t = Transcript::new();
        t.push(BlockKind::User, "first request");
        t.push(BlockKind::Assistant, "first answer");
        t.push(BlockKind::User, "second request");
        let chapters: Vec<u32> = t.blocks().iter().map(|b| b.chapter).collect();
        assert_eq!(chapters, vec![1, 1, 2]);
        assert_eq!(current_chapter(t.blocks()), 2);
        assert_eq!(chapter_range(t.blocks(), 1), Some((0, 2)));
        assert_eq!(chapter_range(t.blocks(), 2), Some((2, 3)));
    }

    #[test]
    fn tool_and_answer_blocks_join_the_request_they_followed() {
        let mut t = Transcript::new();
        t.push(BlockKind::User, "run the query");
        t.buffer_tool_request(
            "bounded_sql_query".into(),
            serde_json::json!({"sql": "SELECT 1"}),
            None,
        );
        assert!(t.buffer_tool_completion("bounded_sql_query", "1 row: a"));
        t.flush_tool_buffer(
            |name, _| vec![format!("→ {name}")],
            |name, summary| format!("✓ {name}: {summary}"),
        );
        t.append_delta(BlockKind::Assistant, "here it is");
        assert!(
            !t.blocks().is_empty(),
            "the request must have produced blocks"
        );
        assert!(
            t.blocks().iter().all(|b| b.chapter == 1),
            "tool and answer blocks join chapter 1: {:?}",
            t.blocks()
                .iter()
                .map(|b| (b.kind, b.chapter))
                .collect::<Vec<_>>()
        );
        assert_eq!(current_chapter(t.blocks()), 1);
    }

    #[test]
    fn blocks_before_any_request_have_an_honest_chapter_value() {
        let mut t = Transcript::new();
        assert_eq!(current_chapter(t.blocks()), 0);
        assert_eq!(chapter_range(t.blocks(), 0), None);
        t.push(BlockKind::System, "welcome");
        assert_eq!(t.blocks()[0].chapter, 0);
        assert_eq!(current_chapter(t.blocks()), 0);
        assert_eq!(chapter_range(t.blocks(), 0), Some((0, 1)));
        assert_eq!(chapter_range(t.blocks(), 1), None);
        t.push(BlockKind::User, "first request");
        assert_eq!(t.blocks()[1].chapter, 1);
        assert_eq!(chapter_range(t.blocks(), 0), Some((0, 1)));
    }

    #[test]
    fn a_partly_evicted_chapter_does_not_panic() {
        let mut t = Transcript::new();
        t.push(BlockKind::User, "first request");
        for i in 0..super::super::MAX_BLOCKS {
            t.push(BlockKind::Tool, format!("tool line {i}"));
        }
        // The opening `User` block evicted; the survivors are mid-chapter.
        assert_eq!(t.blocks()[0].chapter, 1);
        assert_eq!(current_chapter(t.blocks()), 1);
        let (start, end) = chapter_range(t.blocks(), 1).expect("survivors remain");
        assert_eq!((start, end), (0, t.blocks().len()));
        assert_eq!(chapter_range(t.blocks(), 0), None);
        assert_eq!(chapter_range(t.blocks(), 2), None);
        // Numbering derives from the surviving tail, never a recount, so the
        // next request still advances past the evicted chapter.
        t.push(BlockKind::User, "second request");
        assert_eq!(t.blocks().last().unwrap().chapter, 2);
    }

    #[test]
    fn a_queued_prompt_has_no_user_block_and_joins_the_previous_chapter() {
        let mut t = Transcript::new();
        t.push(BlockKind::User, "first request");
        // `submit()` while busy pushes a `System` "Queued …" notice and no
        // `User` block; the prompt's tool and answer blocks arrive later with
        // no `YOU` turn of their own. They join the previous chapter.
        t.push(
            BlockKind::System,
            "Queued — runs as soon as the current request finishes.",
        );
        t.push(BlockKind::Tool, "→ bounded_sql_query");
        t.push(BlockKind::Assistant, "queued answer");
        assert_eq!(
            t.blocks()
                .iter()
                .filter(|b| b.kind == BlockKind::User)
                .count(),
            1,
            "the queued prompt brings no User block of its own"
        );
        assert!(
            t.blocks().iter().all(|b| b.chapter == 1),
            "the queued prompt's blocks join chapter 1"
        );
        assert_eq!(current_chapter(t.blocks()), 1);
        assert_eq!(chapter_range(t.blocks(), 1), Some((0, 4)));
    }

    #[test]
    fn folding_a_chapter_collapses_it_to_one_row_of_its_own_request() {
        let mut t = Transcript::new();
        t.push(BlockKind::User, "count the red orders");
        t.push(BlockKind::Assistant, "the red orders total 42");
        t.push(BlockKind::User, "and the blue ones");
        assert!(t.toggle_chapter(1), "chapter 1 is finished, so it folds");
        let rows = t.wrapped(80);
        assert_eq!(
            rows.len(),
            3,
            "one folded row plus the live chapter's label and body: {rows:?}"
        );
        assert!(
            rows[0].text.contains("count the red orders"),
            "the folded row carries the verbatim request: {:?}",
            rows[0].text
        );
        assert!(!rows[0].is_label, "the folded row is one body row");
    }

    #[test]
    fn toggling_again_restores_every_row() {
        let mut t = Transcript::new();
        t.push(BlockKind::User, "count the red orders");
        t.push(BlockKind::Assistant, "the red orders total 42");
        t.push(BlockKind::User, "and the blue ones");
        let before: Vec<String> = t.wrapped(80).iter().map(|row| row.text.clone()).collect();
        assert!(t.toggle_chapter(1));
        assert!(t.toggle_chapter(1), "the same keystroke reopens it");
        let after: Vec<String> = t.wrapped(80).iter().map(|row| row.text.clone()).collect();
        assert_eq!(before, after);
    }

    #[test]
    fn the_current_chapter_never_folds() {
        let mut t = Transcript::new();
        t.push(BlockKind::User, "first request");
        t.push(BlockKind::Assistant, "first answer");
        assert!(!t.toggle_chapter(1), "the live chapter must stay visible");
        assert!(!t.is_folded(1));
        t.push(BlockKind::User, "second request");
        assert!(
            t.toggle_chapter(1),
            "chapter 1 is finished once a newer request arrives"
        );
        assert!(!t.toggle_chapter(2), "the new live chapter never folds");
    }

    #[test]
    fn a_folded_chapter_still_counts_as_one_row_in_total_lines() {
        let mut t = Transcript::new();
        t.push(BlockKind::User, "count the red orders");
        t.push(BlockKind::Assistant, "the red orders total 42");
        t.push(BlockKind::User, "and the blue ones");
        assert!(t.toggle_chapter(1));
        let rows = t.wrapped(80);
        assert_eq!(t.total_lines(80), rows.len());
        assert_eq!(t.total_lines(80), 3);
    }

    #[test]
    fn a_chapter_without_its_opening_user_block_refuses_to_fold() {
        let mut t = Transcript::new();
        t.push(BlockKind::User, "first request");
        for i in 0..super::super::MAX_BLOCKS {
            t.push(BlockKind::Tool, format!("tool line {i}"));
        }
        assert!(
            t.blocks().iter().all(|b| b.chapter == 1),
            "the opener evicted; only mid-chapter survivors remain"
        );
        assert!(
            !t.toggle_chapter(1),
            "no request text survives, so there is nothing honest to show"
        );
        assert!(!t.is_folded(1));
    }

    #[test]
    fn find_sees_only_the_folded_summary_row() {
        let mut t = Transcript::new();
        t.push(BlockKind::User, "count the red orders");
        t.push(BlockKind::Assistant, "the red orders total 42");
        t.push(BlockKind::User, "and the blue ones");
        assert!(t.toggle_chapter(1));
        assert_eq!(
            t.count_matches("total 42", 80),
            0,
            "hidden rows do not match"
        );
        assert_eq!(
            t.count_matches("red orders", 80),
            1,
            "the folded row still matches its own request"
        );
        assert!(!t.jump_to_match("total 42", 80, 10));
        assert!(t.jump_to_match("red orders", 80, 10));
    }
}
