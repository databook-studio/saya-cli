use super::super::{BlockKind, Transcript};
use super::{chapter_range, current_chapter};
use crate::interactive::tui::application::tests_support::idle_app;

// --- Phase 3, packet 3: automatic fold at the live edge. --------------------
// `App::submit()` (the idle path) folds the chapter that just finished before
// pushing the new `User` block. Guards: fold only while following the tail
// (no selection state exists; scrolled-away means the user is reading), and
// never while an approval modal is open. A material caveat is NOT guarded:
// no block, field, or type marks one — see `auto_fold_finished_chapter` for
// the recorded gap. Only the immediately previous chapter folds; nothing is
// ever unfolded; the busy (queued) path folds nothing.

/// Drives `submit()` with `text` in the input box: `idle_app` leaves the box
/// empty, and `submit()` ignores an empty line, so tests pre-set the draft.
fn submit_text(app: &mut crate::interactive::tui::types::App, text: &str) {
    app.input.set_text(text);
    app.submit();
    app.pending = None;
}

/// Chapter 1 finishes when the second request starts: it folds itself.
#[test]
fn sending_a_new_request_folds_the_chapter_that_just_finished() {
    let mut app = idle_app();
    submit_text(&mut app, "count the red orders");
    app.transcript
        .push(BlockKind::Assistant, "the red orders total 42");
    submit_text(&mut app, "and the blue ones");
    assert!(
        app.transcript.is_folded(1),
        "the finished chapter folds when the next request starts"
    );
    assert!(
        !app.transcript.is_folded(2),
        "the chapter being opened never folds"
    );
}

/// Scrolled away from the tail means the user is reading something: the fold
/// would hide content they are looking at, so nothing folds.
#[test]
fn nothing_folds_while_the_user_has_scrolled_away_from_the_tail() {
    let mut app = idle_app();
    submit_text(&mut app, "count the red orders");
    let long_answer = (1..=30)
        .map(|i| format!("answer line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    app.transcript.push(BlockKind::Assistant, long_answer);
    app.transcript.scroll_up(3, 80, 10);
    assert!(
        !app.transcript.is_following_tail(),
        "precondition: the user has scrolled away"
    );
    submit_text(&mut app, "and the blue ones");
    assert!(
        !app.transcript.is_folded(1),
        "nothing folds while the user is reading history"
    );
}

/// An open approval modal means a decision is unresolved: folding could hide
/// the context the decision is about, so nothing folds.
#[test]
fn nothing_folds_while_an_approval_is_pending() {
    let mut app = idle_app();
    submit_text(&mut app, "count the red orders");
    app.transcript
        .push(BlockKind::Assistant, "the red orders total 42");
    let (respond, _answer) = tokio::sync::oneshot::channel();
    app.request.pending_approval = Some(crate::interactive::tui::types::PendingApproval {
        tool: "bounded_sql_query".into(),
        detail: None,
        grant: None,
        respond,
    });
    submit_text(&mut app, "and the blue ones");
    assert!(
        !app.transcript.is_folded(1),
        "nothing folds while a decision is unresolved"
    );
}

/// The busy path queues the prompt with a `System` notice and no `User`
/// block: no new chapter opens, so there is nothing to fold past.
#[test]
fn a_queued_prompt_folds_nothing() {
    use crate::interactive::tui::application::tests_support::in_flight_task;
    let mut app = idle_app();
    app.transcript.push(BlockKind::User, "count the red orders");
    app.transcript
        .push(BlockKind::Assistant, "the red orders total 42");
    app.transcript.push(BlockKind::User, "and the blue ones");
    assert!(app.transcript.toggle_chapter(1));
    // A second command arrives while a query runs: the busy path holds it.
    app.sql_task = Some(in_flight_task());
    assert!(app.is_busy(), "precondition: the app is busy");
    app.input.set_text("and the green ones");
    app.submit();
    assert_eq!(
        app.transcript
            .blocks()
            .iter()
            .filter(|b| b.kind == BlockKind::User)
            .count(),
        2,
        "the queued prompt brings no User block of its own"
    );
    assert!(
        app.transcript.is_folded(1),
        "the earlier fold stands, untouched"
    );
    assert!(
        !app.transcript.is_folded(2),
        "the queued prompt folds nothing"
    );
}

/// A chapter the user deliberately reopened stays open: auto-fold inserts,
/// never toggles, so it cannot refold what the user opened.
#[test]
fn a_chapter_the_user_reopened_stays_open_when_the_next_request_starts() {
    let mut app = idle_app();
    app.transcript.push(BlockKind::User, "count the red orders");
    app.transcript
        .push(BlockKind::Assistant, "the red orders total 42");
    app.transcript.push(BlockKind::User, "and the blue ones");
    assert!(app.transcript.toggle_chapter(1), "fold it first");
    assert!(
        app.transcript.toggle_chapter(1),
        "the user reopens it by hand"
    );
    assert!(
        !app.transcript.is_folded(1),
        "precondition: chapter 1 is open again"
    );
    submit_text(&mut app, "and the green ones");
    assert!(
        !app.transcript.is_folded(1),
        "the reopened chapter stays open"
    );
    assert!(
        app.transcript.is_folded(2),
        "the chapter that just finished still folds"
    );
}

/// Auto-fold touches exactly the chapter that just finished: older chapters
/// keep whatever state they had — folded ones stay folded, open ones stay
/// open. A walk-back that folded everything older would fold chapter 2 here.
#[test]
fn only_the_previous_chapter_folds_not_everything_older() {
    let mut app = idle_app();
    app.transcript.push(BlockKind::User, "first request");
    app.transcript.push(BlockKind::Assistant, "first answer");
    app.transcript.push(BlockKind::User, "second request");
    app.transcript.push(BlockKind::Assistant, "second answer");
    app.transcript.push(BlockKind::User, "third request");
    assert!(app.transcript.toggle_chapter(1), "chapter 1 folded earlier");
    assert!(
        !app.transcript.is_folded(2),
        "precondition: chapter 2 is open"
    );
    submit_text(&mut app, "fourth request");
    assert!(app.transcript.is_folded(1), "the earlier fold stands");
    assert!(
        !app.transcript.is_folded(2),
        "older open chapters are not folded back"
    );
    assert!(
        app.transcript.is_folded(3),
        "only the chapter that just finished folds"
    );
    assert!(
        !app.transcript.is_folded(4),
        "the chapter being opened never folds"
    );
}

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
