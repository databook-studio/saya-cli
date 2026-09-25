//! Contract tests for the session task list in `saya_types::session_tasks`.
//!
//! A session task list is conversation metadata — it binds no authority — but
//! it is still model-proposed, untrusted input, so it carries the run plan's
//! discipline: validating constructors keep hand-built lists honest, and
//! `validate()` re-checks every bound itself for lists arriving as JSON.

use saya_types::{
    MAX_SESSION_TASKS, MAX_TASK_NOTE_CHARS, MAX_TASK_TITLE_CHARS, SessionTask, SessionTaskError,
    SessionTaskList, TaskStatus,
};

fn task(title: &str, status: TaskStatus) -> SessionTask {
    SessionTask::new(title, status).unwrap()
}

fn noted(title: &str, status: TaskStatus, note: &str) -> SessionTask {
    SessionTask::with_note(title, status, Some(note)).unwrap()
}

// ---------------------------------------------------------------------------
// The list bound: at most MAX_SESSION_TASKS tasks
// ---------------------------------------------------------------------------

#[test]
fn the_list_admits_exactly_max_tasks_and_refuses_one_more() {
    let full: Vec<SessionTask> = (0..MAX_SESSION_TASKS)
        .map(|i| task(&format!("task {i}"), TaskStatus::Pending))
        .collect();
    assert_eq!(full.len(), 32, "the bound pins the scale: {full:?}");
    SessionTaskList::new(full).expect("exactly 32 tasks is inside the bound");

    let over: Vec<SessionTask> = (0..=MAX_SESSION_TASKS)
        .map(|i| task(&format!("task {i}"), TaskStatus::Pending))
        .collect();
    let error = SessionTaskList::new(over).unwrap_err();
    assert!(
        matches!(error, SessionTaskError::TooManyTasks(33)),
        "one past the bound refuses with the count: {error:?}"
    );
}

#[test]
fn validate_rejects_an_overlong_list_that_skipped_the_constructor() {
    let mut tasks = String::new();
    for i in 0..=MAX_SESSION_TASKS {
        if i > 0 {
            tasks.push(',');
        }
        tasks.push_str(&format!(r#"{{"title":"task {i}","status":"pending"}}"#));
    }
    let list: SessionTaskList = serde_json::from_str(&format!(r#"{{"tasks":[{tasks}]}}"#)).unwrap();
    assert!(
        matches!(list.validate(), Err(SessionTaskError::TooManyTasks(33))),
        "validate() itself must catch the overlong list"
    );
}

// ---------------------------------------------------------------------------
// The title bound: non-empty after trimming, at most MAX_TASK_TITLE_CHARS
// ---------------------------------------------------------------------------

#[test]
fn the_title_admits_exactly_max_chars_and_refuses_one_more() {
    let max = "a".repeat(MAX_TASK_TITLE_CHARS);
    assert_eq!(max.chars().count(), 200, "the bound pins the scale");
    task(&max, TaskStatus::Pending);

    let over = "a".repeat(MAX_TASK_TITLE_CHARS + 1);
    let error = SessionTask::new(over, TaskStatus::Pending).unwrap_err();
    assert!(
        matches!(error, SessionTaskError::TitleTooLong),
        "one past the bound refuses: {error:?}"
    );
}

#[test]
fn validate_rejects_an_overlong_title_that_skipped_the_constructor() {
    let long = "a".repeat(MAX_TASK_TITLE_CHARS + 1);
    let list: SessionTaskList = serde_json::from_str(&format!(
        r#"{{"tasks":[{{"title":"{long}","status":"pending"}}]}}"#
    ))
    .unwrap();
    assert!(
        matches!(list.validate(), Err(SessionTaskError::TitleTooLong)),
        "validate() itself must catch the overlong title"
    );
}

#[test]
fn empty_and_whitespace_only_titles_are_refused() {
    for title in ["", "   ", "\t \n "] {
        let error = SessionTask::new(title, TaskStatus::Pending).unwrap_err();
        assert!(
            matches!(error, SessionTaskError::EmptyTitle),
            "no usable text refuses: {title:?} gave {error:?}"
        );
    }
}

#[test]
fn validate_rejects_an_empty_title_that_skipped_the_constructor() {
    let list: SessionTaskList =
        serde_json::from_str(r#"{"tasks":[{"title":"   ","status":"pending"}]}"#).unwrap();
    assert!(
        matches!(list.validate(), Err(SessionTaskError::EmptyTitle)),
        "validate() itself must catch the blank title"
    );
}

// ---------------------------------------------------------------------------
// The note bound: at most MAX_TASK_NOTE_CHARS
// ---------------------------------------------------------------------------

#[test]
fn the_note_admits_exactly_max_chars_and_refuses_one_more() {
    let max = "n".repeat(MAX_TASK_NOTE_CHARS);
    assert_eq!(max.chars().count(), 512, "the bound pins the scale");
    noted("task", TaskStatus::Pending, &max);

    let over = "n".repeat(MAX_TASK_NOTE_CHARS + 1);
    let error = SessionTask::with_note("task", TaskStatus::Pending, Some(&over)).unwrap_err();
    assert!(
        matches!(error, SessionTaskError::NoteTooLong),
        "one past the bound refuses: {error:?}"
    );
}

// ---------------------------------------------------------------------------
// At most one InProgress: the rule that makes the list mean something
// ---------------------------------------------------------------------------

#[test]
fn zero_or_one_in_progress_is_fine_two_is_refused() {
    SessionTaskList::new(vec![
        task("one", TaskStatus::Pending),
        task("two", TaskStatus::Done),
    ])
    .expect("zero in progress is fine");
    SessionTaskList::new(vec![
        task("one", TaskStatus::InProgress),
        task("two", TaskStatus::Pending),
    ])
    .expect("one in progress is fine");

    let error = SessionTaskList::new(vec![
        task("one", TaskStatus::InProgress),
        task("two", TaskStatus::InProgress),
    ])
    .unwrap_err();
    assert!(
        matches!(error, SessionTaskError::TooManyInProgress(2)),
        "two in progress refuses with the count: {error:?}"
    );
}

#[test]
fn validate_rejects_two_in_progress_that_skipped_the_constructor() {
    let list: SessionTaskList = serde_json::from_str(
        r#"{"tasks":[{"title":"one","status":"in_progress"},{"title":"two","status":"in_progress"}]}"#,
    )
    .unwrap();
    assert!(
        matches!(list.validate(), Err(SessionTaskError::TooManyInProgress(2))),
        "validate() itself must catch the double in-progress"
    );
}

// ---------------------------------------------------------------------------
// No control characters in either text field
// ---------------------------------------------------------------------------

#[test]
fn control_characters_are_refused_in_title_and_note() {
    let error = SessionTask::new("title\u{0007}bell", TaskStatus::Pending).unwrap_err();
    assert!(
        matches!(error, SessionTaskError::TitleControlCharacter),
        "a control char in the title refuses: {error:?}"
    );
    let error =
        SessionTask::with_note("task", TaskStatus::Pending, Some("note\nnewline")).unwrap_err();
    assert!(
        matches!(error, SessionTaskError::NoteControlCharacter),
        "a control char in the note refuses: {error:?}"
    );
    let error =
        SessionTask::with_note("task", TaskStatus::Pending, Some("note\u{0}nul")).unwrap_err();
    assert!(
        matches!(error, SessionTaskError::NoteControlCharacter),
        "a NUL in the note refuses: {error:?}"
    );
}

#[test]
fn validate_rejects_control_characters_that_skipped_the_constructor() {
    let list: SessionTaskList =
        serde_json::from_str(r#"{"tasks":[{"title":"ok","status":"pending","note":"a\rb"}]}"#)
            .unwrap();
    assert!(
        matches!(list.validate(), Err(SessionTaskError::NoteControlCharacter)),
        "validate() itself must catch the control char"
    );
}

// ---------------------------------------------------------------------------
// Serialization is snake_case and round-trips; an empty list is valid
// ---------------------------------------------------------------------------

#[test]
fn task_status_serializes_snake_case() {
    assert_eq!(
        serde_json::to_string(&TaskStatus::Pending).unwrap(),
        "\"pending\""
    );
    assert_eq!(
        serde_json::to_string(&TaskStatus::InProgress).unwrap(),
        "\"in_progress\""
    );
    assert_eq!(
        serde_json::to_string(&TaskStatus::Done).unwrap(),
        "\"done\""
    );
    for (word, status) in [
        ("pending", TaskStatus::Pending),
        ("in_progress", TaskStatus::InProgress),
        ("done", TaskStatus::Done),
    ] {
        let back: TaskStatus = serde_json::from_str(&format!("\"{word}\"")).unwrap();
        assert_eq!(back, status, "{word} must parse back");
    }
}

#[test]
fn the_list_round_trips_through_serde() {
    let list = SessionTaskList::new(vec![
        noted(
            "profile the tables",
            TaskStatus::InProgress,
            "halfway there",
        ),
        task("write the report", TaskStatus::Pending),
    ])
    .unwrap();
    let json = serde_json::to_string(&list).unwrap();
    assert!(
        json.contains("\"in_progress\""),
        "the status keeps its snake_case spelling: {json}"
    );
    let back: SessionTaskList = serde_json::from_str(&json).unwrap();
    assert_eq!(list, back);
    back.validate().expect("a round-tripped list stays valid");
}

#[test]
fn an_empty_list_is_valid() {
    let list = SessionTaskList::new(Vec::new()).expect("empty is the starting state");
    assert!(list.tasks.is_empty());
    list.validate().expect("an empty list validates");
    assert_eq!(serde_json::to_string(&list).unwrap(), r#"{"tasks":[]}"#);
    let back: SessionTaskList = serde_json::from_str(r#"{"tasks":[]}"#).unwrap();
    assert_eq!(list, back);
}
