use super::*;

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
