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

#[path = "session_tasks/control_characters.rs"]
mod control_characters;
#[path = "session_tasks/in_progress_rule.rs"]
mod in_progress_rule;
#[path = "session_tasks/list_bound.rs"]
mod list_bound;
#[path = "session_tasks/note_bound.rs"]
mod note_bound;
#[path = "session_tasks/serialization.rs"]
mod serialization;
#[path = "session_tasks/title_bound.rs"]
mod title_bound;
