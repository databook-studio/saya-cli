use super::super::BlockKind;
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
