/// The additive-field rule for session files: a new persisted field stays
/// `#[serde(default)]` at `SESSION_VERSION` 2 — absence deserializes to the
/// old behaviour — so a pre-slice record resumes without error and without
/// the version-skew fallback rewriting its live fields.
#[test]
fn the_mode_is_an_additive_field_at_the_current_session_version() {
    assert_eq!(saya_store::SESSION_VERSION, 2);
    let record: RedactedSession = serde_json::from_str(
        r#"{"version":2,"id":"old","profile_names":[],"turns":[],"messages":[]}"#,
    )
    .expect("a record without the mode field deserializes");
    assert!(
        record.agent_mode.is_empty(),
        "a pre-slice record carries no mode"
    );
}

/// A task list survives the persist/resume round trip identical: the list
/// rides the record the way the mode does, and the resume restores it behind
/// the list's own `validate()` gate.
#[test]
fn a_task_list_round_trips_through_persist_and_resume() {
    use saya_types::{SessionTask, SessionTaskList, TaskStatus};
    let root = std::env::temp_dir().join(format!("saya-tasks-resume-{}", std::process::id()));
    let store = FsSessionStore::new(&root);
    let mut state = crate::SessionState::new("tasked", None, "m");
    state.task_list = SessionTaskList::new(vec![
        SessionTask::with_note(
            "profile the tables",
            TaskStatus::InProgress,
            Some("halfway"),
        )
        .unwrap(),
        SessionTask::new("write the report", TaskStatus::Pending).unwrap(),
    ])
    .unwrap();
    super::block_on(store.save(state.redacted())).unwrap();
    let resumed = load_session(
        &store,
        &cli(),
        &SessionDefaults {
            provider: "ollama".into(),
            model: "m".into(),
            allow_data_sharing: false,
            approval_mode: "ask".into(),
        },
    )
    .unwrap();
    assert_eq!(resumed.task_list, state.task_list);
    std::fs::remove_dir_all(root).unwrap();
}

/// A session file written before the task-list field existed resumes with an
/// empty list: the field is `#[serde(default)]`, so absence deserializes to
/// the starting state. The fixture is raw JSON with no task-list key, so
/// deserialization — not struct construction — proves the old record loads.
#[test]
fn a_session_file_without_the_task_list_field_resumes_empty() {
    let root = std::env::temp_dir().join(format!("saya-tasks-legacy-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("old.json"),
        r#"{"version":2,"id":"old","profile_names":[],"turns":[],"messages":[]}"#,
    )
    .unwrap();
    let state = load_session(
        &FsSessionStore::new(&root),
        &cli(),
        &SessionDefaults {
            provider: "ollama".into(),
            model: "m".into(),
            allow_data_sharing: false,
            approval_mode: "ask".into(),
        },
    )
    .unwrap();
    assert!(
        state.task_list.is_empty(),
        "a pre-slice record resumes with an empty list"
    );
    std::fs::remove_dir_all(root).unwrap();
}

/// A record whose stored list fails `validate()` — hand-edited, or written
/// by an older buggy build — resumes empty rather than failing the session:
/// a corrupt todo list must never make a session unopenable. The fixture is
/// raw JSON with two tasks in progress, so deserialization proves the resume
/// path validates the stored list instead of trusting it.
#[test]
fn a_session_file_with_an_invalid_task_list_resumes_empty_without_error() {
    let root = std::env::temp_dir().join(format!("saya-tasks-invalid-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("bad.json"),
        r#"{"version":2,"id":"bad","profile_names":[],"turns":[],"messages":[],"task_list":{"tasks":[{"title":"one","status":"in_progress"},{"title":"two","status":"in_progress"}]}}"#,
    )
    .unwrap();
    let state = load_session(
        &FsSessionStore::new(&root),
        &cli(),
        &SessionDefaults {
            provider: "ollama".into(),
            model: "m".into(),
            allow_data_sharing: false,
            approval_mode: "ask".into(),
        },
    )
    .expect("an invalid stored list must not fail the resume");
    assert!(
        state.task_list.is_empty(),
        "an invalid stored list resumes empty"
    );
    std::fs::remove_dir_all(root).unwrap();
}

/// The additive-field rule holds for the task list too: it stays
/// `#[serde(default)]` at `SESSION_VERSION` 2 — absence deserializes to the
/// empty list — so no version bump is owed, exactly the `agent_mode`
/// precedent.
#[test]
fn the_task_list_is_an_additive_field_at_the_current_session_version() {
    assert_eq!(saya_store::SESSION_VERSION, 2);
    let record: RedactedSession = serde_json::from_str(
        r#"{"version":2,"id":"old","profile_names":[],"turns":[],"messages":[]}"#,
    )
    .expect("a record without the task-list field deserializes");
    assert!(
        record.task_list.is_empty(),
        "a pre-slice record carries no tasks"
    );
}
