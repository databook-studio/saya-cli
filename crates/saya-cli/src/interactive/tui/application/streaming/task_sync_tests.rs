//! Accepted task writes survive a provider error at the TUI turn boundary.

use super::*;
use crate::interactive::session::runtime::SessionRuntime;
use saya_agent::{ApprovalPolicy, ToolExecutor};
use saya_types::{SessionTask, SessionTaskList, TaskStatus};

#[tokio::test]
async fn a_task_write_survives_an_error_settlement() {
    let root = std::env::temp_dir().join(format!(
        "saya-tui-task-sync-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("the task-sync root");
    let runtime = unused_runtime();
    let mut session = SessionRuntime::acquire(
        &runtime,
        None,
        false,
        None,
        "tui-task-sync",
        ApprovalPolicy::Ask,
        &root,
    )
    .expect("the session runtime acquires");
    let (mut app, mut state) = (idle_app(), SessionState::new("s1", None, "test-model"));
    let accepted = SessionTaskList::new(vec![
        SessionTask::new("keep the task", TaskStatus::InProgress).expect("a valid task"),
    ])
    .expect("valid task list");
    crate::interactive::session::tasks::TasksSet::new(app.session.tasks())
        .execute(
            "tasks_set",
            serde_json::json!({"tasks": accepted.tasks.clone()}),
        )
        .await
        .expect("the task write is accepted");

    let (tx, rx) = unbounded_channel();
    tx.send(StreamMsg::Done(Err("provider failed".into())))
        .expect("the failed turn is queued");
    app.request.stream = Some(Stream {
        rx,
        cancel: CancellationToken::new(),
        prompt: "question".into(),
        question_scope: None,
    });

    assert!(app.drain_stream_with_session(&mut state, &mut session));
    assert_eq!(state.task_list, accepted);

    drop(session);
    let _ = std::fs::remove_dir_all(root);
}
