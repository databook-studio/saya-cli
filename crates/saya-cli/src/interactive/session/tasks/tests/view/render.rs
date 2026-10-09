//! The `/tasks` view rendering: the exact strings below are the user-visible
//! contract for empty, mixed, fully-done, and noteless lists, and the
//! worst-case render stays within the stated ceiling.

use crate::interactive::session::tasks::view::{TASKS_VIEW_CEILING, render_tasks_view};
use saya_types::{SessionTask, SessionTaskList, TaskStatus};

/// `/tasks` on an empty list says there is nothing — the same words the
/// `/tasks clear` answer's neighbourhood already uses for absence, so an
/// empty list after a clear reads as the cleared state.
#[test]
fn tasks_on_an_empty_list_names_the_absence() {
    let list = SessionTaskList::default();
    assert_eq!(
        render_tasks_view(&list),
        "No tasks are being tracked.",
        "an empty list must say so in exactly these words"
    );
}

/// `/tasks` on a mixed list: the header carries the done count, each task
/// one line with its marker, the note after an em dash only when present.
#[test]
fn tasks_on_a_mixed_list_renders_header_markers_and_notes() {
    let list = SessionTaskList::new(vec![
        SessionTask::new("Survey orders schema", TaskStatus::Done).unwrap(),
        SessionTask::with_note(
            "Migrate line_items",
            TaskStatus::InProgress,
            Some("waiting on the backfill script"),
        )
        .unwrap(),
        SessionTask::new("Backfill discounts", TaskStatus::Pending).unwrap(),
    ])
    .unwrap();
    assert_eq!(
        render_tasks_view(&list),
        "Tasks (1/3 done):\n  [x] Survey orders schema\n  [>] Migrate line_items \u{2014} waiting on the backfill script\n  [ ] Backfill discounts",
        "the mixed list must render header, markers, and the note"
    );
}

/// `/tasks` when every task is done: the header reads full, every line
/// marked done.
#[test]
fn tasks_when_everything_is_done_reads_full() {
    let list = SessionTaskList::new(vec![
        SessionTask::new("Survey orders schema", TaskStatus::Done).unwrap(),
        SessionTask::new("Backfill discounts", TaskStatus::Done).unwrap(),
    ])
    .unwrap();
    assert_eq!(
        render_tasks_view(&list),
        "Tasks (2/2 done):\n  [x] Survey orders schema\n  [x] Backfill discounts",
        "a fully-done list must read full with done markers"
    );
}

/// A task without a note renders no trailing separator — the line ends at
/// the title.
#[test]
fn a_task_without_a_note_renders_no_trailing_separator() {
    let list = SessionTaskList::new(vec![
        SessionTask::new("Backfill discounts", TaskStatus::Pending).unwrap(),
    ])
    .unwrap();
    let rendered = render_tasks_view(&list);
    assert!(
        rendered.ends_with("Backfill discounts"),
        "a noteless task must end at its title, got:\n{rendered}"
    );
    assert!(
        !rendered.contains('\u{2014}'),
        "a noteless list must carry no em dash at all, got:\n{rendered}"
    );
}

/// Rendering is bounded: a worst-case list (32 tasks, max-length titles and
/// notes) renders without panic and within the stated ceiling — 32 KiB,
/// pinned here in the test, not just in the source.
#[test]
fn the_worst_case_view_renders_within_the_stated_ceiling() {
    use saya_types::{
        MAX_SESSION_TASKS, MAX_TASK_NOTE_CHARS, MAX_TASK_TITLE_CHARS, SessionTask, SessionTaskList,
    };
    let title = "t".repeat(MAX_TASK_TITLE_CHARS);
    let note = "n".repeat(MAX_TASK_NOTE_CHARS);
    let pad = MAX_TASK_TITLE_CHARS - 3;
    let tasks: Vec<SessionTask> = (0..MAX_SESSION_TASKS)
        .map(|i| {
            let status = if i == 0 {
                TaskStatus::InProgress
            } else {
                TaskStatus::Pending
            };
            SessionTask::with_note(
                format!("{} {i:02}", &title[..pad]),
                status,
                Some(note.as_str()),
            )
            .unwrap()
        })
        .collect();
    let list = SessionTaskList::new(tasks).unwrap();
    let rendered = render_tasks_view(&list);
    assert!(
        rendered.len() <= TASKS_VIEW_CEILING,
        "worst-case view {} exceeds ceiling {TASKS_VIEW_CEILING}",
        rendered.len()
    );
    assert_eq!(
        TASKS_VIEW_CEILING, 32_768,
        "the ceiling is stated here: 32 KiB — {TASKS_VIEW_CEILING}"
    );
}

/// Character-count validation also admits four-byte Unicode, so the user
/// view must enforce its advertised limit in UTF-8 bytes and say when it
/// omits the remainder.
#[test]
fn multibyte_task_text_is_byte_bounded_and_truthfully_truncated() {
    use saya_types::{MAX_SESSION_TASKS, MAX_TASK_NOTE_CHARS, MAX_TASK_TITLE_CHARS};
    let title = "🧪".repeat(MAX_TASK_TITLE_CHARS);
    let note = "🧪".repeat(MAX_TASK_NOTE_CHARS);
    let tasks = (0..MAX_SESSION_TASKS)
        .map(|i| {
            SessionTask::with_note(
                title.as_str(),
                if i == 0 {
                    TaskStatus::InProgress
                } else {
                    TaskStatus::Pending
                },
                Some(note.as_str()),
            )
            .unwrap()
        })
        .collect();
    let list = SessionTaskList::new(tasks).unwrap();
    let rendered = render_tasks_view(&list);
    assert!(
        rendered.len() <= TASKS_VIEW_CEILING,
        "{} bytes exceeds {TASKS_VIEW_CEILING}",
        rendered.len()
    );
    assert!(
        rendered.contains("[truncated"),
        "omitted task content must be disclosed: {rendered}"
    );
    assert!(rendered.contains("Tasks (0/32 done):"));
    assert!(rendered.contains("[>]"));
}
