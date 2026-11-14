use super::*;

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
