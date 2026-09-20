//! Chapters: one request and everything it produced, held together.
//!
//! Phase 3, packet 1 creates the data only: every [`Block`] carries a
//! chapter number, and a new chapter begins at each [`BlockKind::User`]
//! block. Nothing renders differently; a later packet adds the index and
//! the fold toggle on top of [`current_chapter`] and [`chapter_range`].
//!
//! Blocks pushed before any `User` block exists (the welcome message, a
//! resumed-session divider) carry chapter `0` (`PRE_CHAPTER`): a plain
//! integer every read site can compare and group by without unwrapping,
//! sorting before all real chapters.
//!
//! [`Block`]: super::Block
//! [`BlockKind::User`]: super::BlockKind::User

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
}
