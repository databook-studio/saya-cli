//! The user's view of the session task list: what `/tasks` prints and what
//! the status line abbreviates it to.
//!
//! The model owns the list's content (`tasks_set` writes it); this module
//! only renders it. Two shapes: the full listing [`render_tasks_view`] for
//! `/tasks`, and the compact [`tasks_summary`] for the status line. Both
//! read the same counts, so the header and the status segment agree.
//!
//! Titles and notes are model-proposed and untrusted, but they render here
//! as plain lines in the user's own terminal — the same trust posture as
//! echoing the conversation back — not as instructions to any model.

use saya_types::{SessionTaskList, TaskStatus};

/// The ceiling a rendered `/tasks` view never exceeds: 32 tasks ×
/// (200-char title + 512-char note + the line's own markers and separators)
/// plus the header — 32 KiB, pinned by the worst-case test (which reads
/// this constant, so production and test share one number).
#[cfg(any(test, doctest))]
pub(crate) const TASKS_VIEW_CEILING: usize = 32_768;

/// Renders the list for `/tasks`: a `Tasks (done/total done):` header, one
/// line per task — `[x]` done, `[>]` in progress, `[ ]` pending — the note
/// after an em dash only when present. An empty list names the absence.
pub(crate) fn render_tasks_view(list: &SessionTaskList) -> String {
    if list.is_empty() {
        return "No tasks are being tracked.".to_string();
    }
    let done = list
        .tasks
        .iter()
        .filter(|task| task.status == TaskStatus::Done)
        .count();
    let mut out = format!("Tasks ({done}/{} done):", list.tasks.len());
    for task in &list.tasks {
        out.push_str("\n  ");
        out.push_str(match task.status {
            TaskStatus::Done => "[x]",
            TaskStatus::InProgress => "[>]",
            // `TaskStatus` is `#[non_exhaustive]`: a future status renders
            // as pending rather than failing the view.
            _ => "[ ]",
        });
        out.push(' ');
        out.push_str(&task.title);
        if let Some(note) = task.note.as_deref() {
            out.push_str(" \u{2014} ");
            out.push_str(note);
        }
    }
    out
}

/// The status-line abbreviation — `tasks: done/total` — or `None` when the
/// list is empty, so the segment is absent rather than reading `tasks: 0/0`.
pub(crate) fn tasks_summary(list: &SessionTaskList) -> Option<String> {
    if list.is_empty() {
        return None;
    }
    let done = list
        .tasks
        .iter()
        .filter(|task| task.status == TaskStatus::Done)
        .count();
    Some(format!("tasks: {done}/{}", list.tasks.len()))
}
