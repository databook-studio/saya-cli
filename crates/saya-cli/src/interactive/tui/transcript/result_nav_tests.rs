//! Phase 8 packet 3: reaching an old result without opening every chapter.
//!
//! A result is a `BlockKind::Table` block, addressed by its place in block
//! order — no id, ordinal, or jump target is stored. These drive the real key
//! path (`handle_key`) against the app's last rendered viewport, so the arm
//! cannot pass by calling the index directly.

use crate::interactive::tui::application::tests_support::idle_app;
use crate::interactive::tui::keys::handle_key;
use crate::interactive::tui::types::{App, PendingApproval};
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use super::BlockKind;

/// Three results, each opening its own chapter, with a 4-row viewport.
fn app_with_results() -> App {
    let mut app = idle_app();
    app.viewport.set((80, 4));
    app.transcript.push(BlockKind::User, "first question");
    app.transcript.push(BlockKind::Table, "t1-head\nt1-tail");
    app.transcript.push(BlockKind::User, "second question");
    app.transcript.push(BlockKind::Table, "t2-head\nt2-tail");
    app.transcript.push(BlockKind::User, "third question");
    app.transcript.push(BlockKind::Table, "t3-head\nt3-tail");
    app
}

fn painted(app: &App) -> Vec<String> {
    app.transcript
        .view(80, 4)
        .into_iter()
        .map(|row| row.text)
        .collect()
}

#[test]
fn stepping_back_lands_on_the_previous_result() {
    let mut app = app_with_results();
    assert!(app.transcript.is_following_tail());
    handle_key(&mut app, KeyCode::Up, KeyModifiers::ALT);
    let rows = painted(&app);
    assert_eq!(
        rows[0], "RESULT",
        "lands on the result's label row: {rows:?}"
    );
    assert_eq!(rows[1], "t2-head", "with its rows beneath: {rows:?}");
    assert!(
        !rows.iter().any(|r| r == "t3-head"),
        "the newest result is left behind: {rows:?}"
    );
}

#[test]
fn stepping_forward_lands_on_the_next_result() {
    let mut app = app_with_results();
    handle_key(&mut app, KeyCode::Up, KeyModifiers::ALT);
    handle_key(&mut app, KeyCode::Down, KeyModifiers::ALT);
    let rows = painted(&app);
    assert!(
        rows.iter().any(|r| r == "t3-head") && rows.iter().any(|r| r == "t3-tail"),
        "forward lands on the next result: {rows:?}"
    );
    assert!(
        !rows.iter().any(|r| r == "t1-head"),
        "it does not wrap to the first result: {rows:?}"
    );
}

#[test]
fn a_folded_chapter_opens_when_its_result_is_the_target() {
    let mut app = app_with_results();
    assert!(app.transcript.toggle_chapter(2), "chapter 2 may fold");
    assert!(app.transcript.is_folded(2));
    handle_key(&mut app, KeyCode::Up, KeyModifiers::ALT);
    assert!(
        !app.transcript.is_folded(2),
        "the target chapter unfolds so the result can paint"
    );
    let rows = painted(&app);
    assert!(
        rows.iter().any(|r| r == "t2-head"),
        "and the result's rows are on screen: {rows:?}"
    );
}

#[test]
fn with_no_results_nothing_moves_and_the_user_is_told() {
    let mut app = idle_app();
    app.viewport.set((80, 4));
    app.transcript.push(BlockKind::User, "hello");
    app.transcript.push(BlockKind::Assistant, "no table here");
    handle_key(&mut app, KeyCode::Up, KeyModifiers::ALT);
    assert!(
        app.transcript.is_following_tail(),
        "the viewport did not move"
    );
    let last = app.transcript.blocks().last().expect("a notice was pushed");
    assert_eq!(last.kind, BlockKind::System, "plainly said: {last:?}");
    assert!(
        last.text.to_lowercase().contains("no result"),
        "the notice names the absence: {:?}",
        last.text
    );
}

#[test]
fn stepping_past_the_last_result_stops_there() {
    let mut app = app_with_results();
    let before = app.transcript.blocks().len();
    handle_key(&mut app, KeyCode::Down, KeyModifiers::ALT);
    assert!(app.transcript.is_following_tail(), "stops at the end");
    let rows = painted(&app);
    assert!(
        !rows.iter().any(|r| r == "t1-head"),
        "no silent wrap to the first result: {rows:?}"
    );
    assert_eq!(
        app.transcript.blocks().len(),
        before,
        "a stop is silent — no notice and no block"
    );
}

#[test]
fn result_navigation_reruns_nothing() {
    let mut app = app_with_results();
    let before = app.transcript.blocks().len();
    handle_key(&mut app, KeyCode::Up, KeyModifiers::ALT);
    assert!(
        !app.transcript.is_following_tail(),
        "it stepped away from the tail"
    );
    assert_eq!(
        app.transcript.blocks().len(),
        before,
        "no result block was pushed — it reads what is rendered"
    );
    assert!(app.sql_task.is_none(), "no query was dispatched");
    assert!(app.last_query.is_none(), "no query became the last query");
}

#[test]
fn the_step_key_does_not_disturb_an_approval() {
    let mut app = app_with_results();
    let (respond, mut answer) = tokio::sync::oneshot::channel();
    app.request.pending_approval = Some(PendingApproval {
        tool: "bounded_sql_query".into(),
        detail: Some("SELECT 1".into()),
        grant: None,
        scroll: 0,
        respond,
    });
    handle_key(&mut app, KeyCode::Up, KeyModifiers::ALT);
    assert!(
        app.request.pending_approval.is_some(),
        "the modal keeps the key"
    );
    assert!(
        answer.try_recv().is_err(),
        "no approval answer may be sent by the step key"
    );
    assert!(
        app.transcript.is_following_tail(),
        "the modal swallows the step — nothing jumped"
    );
}

#[test]
fn jumping_to_an_older_result_keeps_the_unseen_count() {
    let mut app = app_with_results();
    app.transcript.scroll_up(2, 80, 4);
    let _ = app.transcript.view(80, 4);
    app.transcript
        .push(BlockKind::Assistant, "a late answer lands");
    let before = app.unseen_new_rows();
    assert!(before > 0, "precondition: rows are unseen below");
    handle_key(&mut app, KeyCode::Up, KeyModifiers::ALT);
    assert_eq!(
        app.unseen_new_rows(),
        before,
        "rows below are still unseen, so the count must persist"
    );
}
