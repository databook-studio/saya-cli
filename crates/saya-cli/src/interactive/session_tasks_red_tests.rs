//! Slice 3+4 red tests: `tasks_set` stores a valid list, bound
//! violations are typed and non-destructive, the tool is advertised wherever
//! the session can use it (including Plan), and the list rides each turn as
//! one user-turn context block — never the system prompt.

use saya_agent::{AgentMode, ApprovalPolicy, SessionPolicy, ToolExecutor};

use crate::interactive::session_tasks::{SessionTasks, TasksSet};

fn valid_args() -> serde_json::Value {
    serde_json::json!({"tasks": [
        {"title": "profile the tables", "status": "in_progress", "note": "halfway"},
        {"title": "write the report", "status": "pending"},
    ]})
}

fn current_titles(tasks: &SessionTasks) -> Vec<String> {
    tasks
        .current()
        .tasks
        .iter()
        .map(|task| task.title.clone())
        .collect()
}

/// `tasks_set` stores a valid list; the session carries it afterwards.
#[tokio::test]
async fn tasks_set_stores_a_valid_list() {
    let tasks = SessionTasks::default();
    let executor = TasksSet::new(tasks.clone());
    let result = executor
        .execute("tasks_set", valid_args())
        .await
        .expect("a valid list stores");
    assert_eq!(result, serde_json::json!({"tasks": 2}));
    assert_eq!(
        current_titles(&tasks),
        vec!["profile the tables", "write the report"]
    );
}

/// An unknown tool name is not this tool: the member refuses it as unknown.
#[tokio::test]
async fn tasks_set_refuses_an_unknown_tool_name() {
    let tasks = SessionTasks::default();
    let executor = TasksSet::new(tasks.clone());
    let error = executor
        .execute("tasks_add", valid_args())
        .await
        .expect_err("another name is unknown");
    assert_eq!(error, saya_agent::ToolError::UnsupportedTool);
    assert!(tasks.current().is_empty());
}

/// A call without the `tasks` array is a shape error, never a stored list.
#[tokio::test]
async fn tasks_set_refuses_a_missing_tasks_array() {
    let tasks = SessionTasks::default();
    let executor = TasksSet::new(tasks.clone());
    let error = executor
        .execute("tasks_set", serde_json::json!({}))
        .await
        .expect_err("no tasks key refuses");
    assert_eq!(error, saya_agent::ToolError::TasksNotArray);
    assert!(tasks.current().is_empty());
}

/// An extra property is refused: the schema is exactly `tasks`, nothing else.
#[tokio::test]
async fn tasks_set_refuses_an_extra_property() {
    let tasks = SessionTasks::default();
    let executor = TasksSet::new(tasks.clone());
    let error = executor
        .execute("tasks_set", serde_json::json!({"tasks": [], "extra": true}))
        .await
        .expect_err("an extra property refuses");
    assert_eq!(error, saya_agent::ToolError::UnsupportedProperty);
    assert!(tasks.current().is_empty());
}

/// Each bound's violation returns a typed error and leaves the stored list
/// unchanged — a rejected write never clears what was there.
#[tokio::test]
async fn tasks_set_violations_are_typed_and_leave_the_stored_list_unchanged() {
    use saya_types::{MAX_SESSION_TASKS, MAX_TASK_NOTE_CHARS, MAX_TASK_TITLE_CHARS};
    let over_title = "t".repeat(MAX_TASK_TITLE_CHARS + 1);
    let over_note = "n".repeat(MAX_TASK_NOTE_CHARS + 1);
    let many: Vec<serde_json::Value> = (0..=MAX_SESSION_TASKS)
        .map(|i| serde_json::json!({"title": format!("task {i}"), "status": "pending"}))
        .collect();
    let control = "bad\u{0007}title";
    let cases: Vec<(serde_json::Value, saya_agent::ToolError)> = vec![
        (
            serde_json::json!({"tasks": many}),
            saya_agent::ToolError::TooManyTasks,
        ),
        (
            serde_json::json!({"tasks": [
                {"title": "one", "status": "in_progress"},
                {"title": "two", "status": "in_progress"},
            ]}),
            saya_agent::ToolError::TooManyInProgress,
        ),
        (
            serde_json::json!({"tasks": [{"title": "", "status": "pending"}]}),
            saya_agent::ToolError::TaskTitleEmpty,
        ),
        (
            serde_json::json!({"tasks": [{"title": over_title, "status": "pending"}]}),
            saya_agent::ToolError::TaskTitleTooLong,
        ),
        (
            serde_json::json!({"tasks": [{"title": control, "status": "pending"}]}),
            saya_agent::ToolError::TaskTitleControl,
        ),
        (
            serde_json::json!({"tasks": [
                {"title": "ok", "status": "pending", "note": over_note},
            ]}),
            saya_agent::ToolError::TaskNoteTooLong,
        ),
        (
            serde_json::json!({"tasks": [
                {"title": "ok", "status": "pending", "note": "bad\u{0007}note"},
            ]}),
            saya_agent::ToolError::TaskNoteControl,
        ),
    ];
    for (arguments, expected) in cases {
        let tasks = SessionTasks::default();
        let executor = TasksSet::new(tasks.clone());
        executor
            .execute("tasks_set", valid_args())
            .await
            .expect("the baseline stores");
        let error = executor
            .execute("tasks_set", arguments.clone())
            .await
            .expect_err("the violation refuses");
        assert_eq!(error, expected, "wrong typed error for {arguments}");
        assert_eq!(
            current_titles(&tasks),
            vec!["profile the tables", "write the report"],
            "a rejected write must not clear what was there: {arguments}"
        );
        // The error text never contains the rejected payload.
        let text = error.to_string();
        for title in arguments
            .get("tasks")
            .and_then(serde_json::Value::as_array)
            .iter()
            .flat_map(|items| items.iter())
            .filter_map(|item| item.get("title").and_then(serde_json::Value::as_str))
            .filter(|title| title.len() > 8)
        {
            assert!(
                !text.contains(title),
                "the error echoed the rejected payload: {text}"
            );
        }
    }
}

/// The tool is advertised under `ask`+prompt, under `bypass`, and under Plan
/// mode — the Plan case is the one this whole design exists for. A
/// read-only session sees it too: session metadata is not what the
/// posture guards.
#[test]
fn tasks_set_is_advertised_under_ask_bypass_and_plan() {
    use crate::interactive::session_tasks_render::tasks_set_definition;
    let definition = tasks_set_definition();
    assert_eq!(definition.name, "tasks_set");
    assert_eq!(
        definition.effect.local_state,
        saya_agent::LocalStateEffect::WriteSession,
        "declared as WriteSession, never workspace, database, or cache"
    );
    assert!(
        !definition.effect.external_side_effect,
        "no external side effect: Plan and read-only can admit it"
    );
    assert!(
        !definition.effect.database_data,
        "no database data: the privacy gate never hides it"
    );
    assert!(
        !definition.effect.requires_approval,
        "auto-allowed wherever reads are: an approval engine that \
         auto-allows it under every policy that permits reads needs no ask"
    );
    assert!(
        definition.read_only,
        "read_only so the completion summary reads as a read, not a write"
    );

    let effect = &definition.effect;
    // Under `ask`+Build the engine asks (the loop then auto-runs: with
    // `requires_approval: false` the decider is never consulted). Read-only
    // allows it under both modes; Plan+bypass allows it (the mode judges
    // what the task may touch, and session metadata is admitted); `never`
    // still denies it. Plan+ask resolves `Ask` — the terminal and the TUI
    // modal share the session's one policy, and `requires_approval: false`
    // means the loop never consults the decider, so the call runs without
    // prompting exactly as under read-only.
    assert_eq!(
        SessionPolicy::new(ApprovalPolicy::Ask).resolve(effect, None),
        saya_agent::ApprovalDecision::Ask,
        "ask hands the call to the frontend; the loop auto-runs it"
    );
    assert_eq!(
        SessionPolicy::new(ApprovalPolicy::ReadOnly).resolve(effect, None),
        saya_agent::ApprovalDecision::Allow,
        "read-only allows a session-metadata write"
    );
    assert_eq!(
        SessionPolicy::new(ApprovalPolicy::ReadOnly)
            .with_agent_mode(AgentMode::Plan)
            .resolve(effect, None),
        saya_agent::ApprovalDecision::Allow,
        "read-only+plan admits session metadata"
    );
    assert_eq!(
        SessionPolicy::new(ApprovalPolicy::Bypass)
            .with_agent_mode(AgentMode::Plan)
            .resolve(effect, None),
        saya_agent::ApprovalDecision::Allow,
        "Plan+bypass: the Plan case this design exists for"
    );
    for mode in [ApprovalPolicy::Never] {
        for agent_mode in [AgentMode::Build, AgentMode::Plan] {
            assert_eq!(
                SessionPolicy::new(mode)
                    .with_agent_mode(agent_mode)
                    .resolve(effect, None),
                saya_agent::ApprovalDecision::Deny { reason: None },
                "never still denies it under {agent_mode:?}"
            );
        }
    }
}
