use super::*;

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
