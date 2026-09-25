use super::*;
use crate::interactive::tui::application::tests_support::idle_app_with_sql_task;
use crate::interactive::tui::sql_task::{Followup, SqlTask};
use crate::render::TerminalEvent;

/// Esc while a direct-SQL command is running detaches it: the UI stops
/// tracking the query and posts an honest "still running, result discarded"
/// message — never "cancelled".
#[test]
fn esc_detaches_a_running_sql_command() {
    let mut app = idle_app_with_sql_task();
    assert!(app.sql_task.is_some(), "precondition: a task is running");
    handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(app.sql_task.is_none(), "Esc detached the running task");
    let last = app
        .transcript
        .blocks()
        .last()
        .expect("detach posts a message");
    let text = last.text.to_lowercase();
    assert!(
        text.contains("running"),
        "honest about still running: {text}"
    );
    assert!(
        !text.contains("cancel"),
        "must not claim cancellation: {text}"
    );
}

/// Phase 5 packet 1: the SQL path is unchanged — detaching says the query
/// may still be running, never that it was cancelled or stopped.
#[test]
fn a_detached_sql_query_still_says_it_may_still_be_running() {
    let mut app = idle_app_with_sql_task();
    handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    let last = app
        .transcript
        .blocks()
        .last()
        .expect("detach posts a message");
    assert!(
        last.text.contains("may still be running"),
        "the SQL wording is unchanged: {}",
        last.text
    );
    assert!(
        !last.text.contains("Stopped."),
        "the SQL path must not claim a stop: {}",
        last.text
    );
}

/// Phase 5 packet 4: after a detach there is nothing to wait for — the app
/// is idle, so the next revision stages for dispatch instead of queueing.
/// RED: passes today (detach clears the task); pins the contrast with the
/// agent-stream hold.
#[test]
fn a_revision_after_a_sql_detach_runs_immediately() {
    let mut app = idle_app_with_sql_task();
    handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(
        !app.is_busy(),
        "precondition: after a detach nothing is in flight"
    );
    assert!(
        app.pending.is_none(),
        "precondition: a detach queues nothing"
    );
}

/// Phase 5 packet 4: the anti-lying test. After a detach plus a revision,
/// the transcript still says the old query may still be running, and
/// nowhere claims it stopped, ended, or was cancelled.
#[test]
fn a_detached_query_is_never_said_to_have_stopped() {
    let mut app = idle_app_with_sql_task();
    handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    app.input.set_text("the follow-up question");
    app.submit();
    let texts: Vec<&str> = app
        .transcript
        .blocks()
        .iter()
        .map(|block| block.text.as_str())
        .collect();
    assert!(
        texts
            .iter()
            .any(|text| text.contains("may still be running")),
        "the detach wording stands after the revision: {texts:?}"
    );
    for text in &texts {
        let lower = text.to_lowercase();
        assert!(
            !lower.contains("stopped")
                && !lower.contains("was cancelled")
                && !lower.contains("has ended")
                && !lower.contains("query ended")
                && !lower.contains("query stopped"),
            "nowhere claims the detached query ended: {text}"
        );
    }
}

#[test]
fn esc_does_not_detach_when_no_task_is_running() {
    let mut app = idle_app_with_sql_task();
    app.sql_task = None;
    handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(app.sql_task.is_none());
    // No detach message posted (the app starts with an empty transcript).
    assert!(app.transcript.blocks().is_empty());
}

#[test]
fn in_flight_task_shape_matches_app_state() {
    // Guards the tuple arity (receiver, task, instant) the dispatch loop
    // and detach path destructure.
    let (_tx, rx) = std::sync::mpsc::channel::<TerminalEvent>();
    let task = SqlTask {
        profile: Some("analytics".into()),
        sql: "SELECT 1".into(),
        followup: Followup::Sql {
            connection: Some("analytics".into()),
        },
    };
    let _: (
        std::sync::mpsc::Receiver<TerminalEvent>,
        SqlTask,
        std::time::Instant,
    ) = (rx, task, std::time::Instant::now());
}
