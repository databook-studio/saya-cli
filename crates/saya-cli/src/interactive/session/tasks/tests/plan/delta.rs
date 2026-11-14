//! The render gate: a request carries the list on the turn after it changed,
//! and nothing on the turns it did not.

use super::*;
use saya_types::{SessionTask, SessionTaskList, TaskStatus};

fn list(title: &str) -> SessionTaskList {
    SessionTaskList::new(vec![
        SessionTask::new(title, TaskStatus::InProgress).expect("a valid task"),
    ])
    .expect("a valid list")
}

/// Two turns with no change between them: the first takes the list, the
/// second takes nothing — and the cell still holds it.
#[test]
fn an_unchanged_list_is_taken_once() {
    let tasks = SessionTasks::default();
    assert!(
        tasks.take_changed().is_none(),
        "a fresh cell renders nothing"
    );
    tasks.replace(list("parse records"));
    assert!(
        tasks.take_changed().is_some(),
        "the turn after the change carries the list"
    );
    assert!(
        tasks.take_changed().is_none(),
        "the next turn carries nothing: the model already has the state"
    );
    assert_eq!(
        tasks.current(),
        list("parse records"),
        "the gate is a render decision, never a delete"
    );
}

/// The session record re-seeds the same list at the start of every turn; a
/// replace that stores an equal list is not a change.
#[test]
fn a_replace_that_stores_an_equal_list_is_not_a_change() {
    let tasks = SessionTasks::default();
    tasks.replace(list("parse records"));
    assert!(tasks.take_changed().is_some());
    tasks.replace(list("parse records"));
    assert!(
        tasks.take_changed().is_none(),
        "the same list re-seeded is not a change"
    );
}

/// A call that changes the list arms the next turn again.
#[test]
fn a_changed_list_is_taken_again() {
    let tasks = SessionTasks::default();
    tasks.replace(list("parse records"));
    assert!(tasks.take_changed().is_some());
    tasks.replace(list("write the report"));
    assert_eq!(
        tasks
            .take_changed()
            .map(|taken| taken.tasks[0].title.clone()),
        Some("write the report".to_owned()),
        "the turn after a change carries the new list"
    );
    assert!(tasks.take_changed().is_none());
}

/// An empty list never renders — not even when it replaced a non-empty one —
/// and a list that comes back is a change again.
#[test]
fn an_empty_list_is_never_taken() {
    let tasks = SessionTasks::default();
    tasks.replace(SessionTaskList::default());
    assert!(tasks.take_changed().is_none(), "empty costs zero tokens");
    tasks.replace(list("parse records"));
    assert!(tasks.take_changed().is_some());
    tasks.replace(SessionTaskList::default());
    assert!(
        tasks.take_changed().is_none(),
        "clearing the list renders nothing"
    );
    tasks.replace(list("parse records"));
    assert!(
        tasks.take_changed().is_some(),
        "the same list restored is a change again"
    );
}
