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
