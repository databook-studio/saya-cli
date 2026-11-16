use super::*;

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
