//! Slice 5 red tests: the user's view of the task list — the `/tasks`
//! rendering the shared dispatcher hands both surfaces, and the `tasks:`
//! status segment the headless header and the TUI bar both read.
//!
//! The dispatcher owns the list's content (the model writes it, `/tasks
//! clear` empties it); this module renders it. The exact strings below are
//! the user-visible contract.

use saya_types::{SessionTask, SessionTaskList, TaskStatus};

use crate::interactive::session_tasks_render::render_tasks_block;
use crate::interactive::session_tasks_view::{
    TASKS_VIEW_CEILING, render_tasks_view, tasks_summary,
};

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
    use saya_types::{MAX_SESSION_TASKS, MAX_TASK_NOTE_CHARS, MAX_TASK_TITLE_CHARS};
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

/// The shared dispatcher shows the list: `/tasks` on an empty session names
/// the absence, on a mixed list renders the exact view, and on a fully-done
/// list reads full.
#[test]
fn tasks_through_apply_renders_empty_mixed_and_done() {
    use crate::interactive::SessionAction;
    use crate::interactive::session_state::SessionState;
    use crate::slash::SlashCommand;

    let mut state = SessionState::new("s1", None, "m");
    let SessionAction::Message(empty) = state.apply(SlashCommand::Tasks(None), &[]) else {
        panic!("expected SessionAction::Message");
    };
    assert_eq!(empty, "No tasks are being tracked.");

    state.task_list = SessionTaskList::new(vec![
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
    let SessionAction::Message(mixed) = state.apply(SlashCommand::Tasks(None), &[]) else {
        panic!("expected SessionAction::Message");
    };
    assert_eq!(
        mixed,
        "Tasks (1/3 done):\n  [x] Survey orders schema\n  [>] Migrate line_items \u{2014} waiting on the backfill script\n  [ ] Backfill discounts"
    );

    state.task_list = SessionTaskList::new(vec![
        SessionTask::new("Survey orders schema", TaskStatus::Done).unwrap(),
        SessionTask::new("Backfill discounts", TaskStatus::Done).unwrap(),
    ])
    .unwrap();
    let SessionAction::Message(done) = state.apply(SlashCommand::Tasks(None), &[]) else {
        panic!("expected SessionAction::Message");
    };
    assert_eq!(
        done,
        "Tasks (2/2 done):\n  [x] Survey orders schema\n  [x] Backfill discounts"
    );
}

/// `/tasks clear` answers exactly `Task list cleared.`, empties the stored
/// list, and leaves the next turn with no context block to inject.
#[test]
fn tasks_clear_empties_the_list_and_stops_the_next_turn_block() {
    use crate::interactive::SessionAction;
    use crate::interactive::session_state::SessionState;
    use crate::slash::SlashCommand;

    let mut state = SessionState::new("s1", None, "m");
    state.task_list = SessionTaskList::new(vec![
        SessionTask::new("Survey orders schema", TaskStatus::Done).unwrap(),
    ])
    .unwrap();
    let SessionAction::Message(cleared) =
        state.apply(SlashCommand::Tasks(Some("clear".into())), &[])
    else {
        panic!("expected SessionAction::Message");
    };
    assert_eq!(cleared, "Task list cleared.");
    assert!(
        state.task_list.is_empty(),
        "clear must empty the stored list"
    );
    assert!(
        render_tasks_block(&state.task_list).is_none(),
        "the next turn must inject no context block after a clear"
    );
    let SessionAction::Message(shown) = state.apply(SlashCommand::Tasks(None), &[]) else {
        panic!("expected SessionAction::Message");
    };
    assert_eq!(shown, "No tasks are being tracked.");
}

/// The status segment reads `tasks: 2/5` while anything is tracked, and is
/// absent for an empty list.
#[test]
fn the_tasks_segment_counts_done_over_total_and_vanishes_when_empty() {
    let list = SessionTaskList::new(vec![
        SessionTask::new("one", TaskStatus::Done).unwrap(),
        SessionTask::new("two", TaskStatus::Done).unwrap(),
        SessionTask::new("three", TaskStatus::Pending).unwrap(),
        SessionTask::new("four", TaskStatus::Pending).unwrap(),
        SessionTask::new("five", TaskStatus::Pending).unwrap(),
    ])
    .unwrap();
    assert_eq!(tasks_summary(&list).as_deref(), Some("tasks: 2/5"));
    assert_eq!(
        tasks_summary(&SessionTaskList::default()),
        None,
        "an empty list must yield no segment at all"
    );
}

/// The line-loop header and the TUI view agree on the segment — the
/// anti-drift property: the same state yields the same words on both
/// surfaces, and an empty list hides the segment on both.
#[test]
fn the_line_loop_status_and_the_view_agree() {
    use crate::interactive::session_prompt::{status_line, status_segments};
    use crate::interactive::session_state::SessionState;

    let mut state = SessionState::new("s1", Some(String::from("analytics")), "m");
    state.task_list = SessionTaskList::new(vec![
        SessionTask::new("one", TaskStatus::Done).unwrap(),
        SessionTask::new("two", TaskStatus::Done).unwrap(),
        SessionTask::new("three", TaskStatus::Pending).unwrap(),
        SessionTask::new("four", TaskStatus::Pending).unwrap(),
        SessionTask::new("five", TaskStatus::Pending).unwrap(),
    ])
    .unwrap();
    let line = status_line(&state);
    let view = status_segments(&state);
    assert!(
        line.contains("tasks: 2/5"),
        "the line-loop status must show the segment: {line}"
    );
    assert_eq!(
        view.task_summary.as_deref(),
        Some("tasks: 2/5"),
        "the TUI view must mirror the header"
    );

    let bare = SessionState::new("s1", Some(String::from("analytics")), "m");
    let bare_line = status_line(&bare);
    assert!(
        !bare_line.contains("tasks:"),
        "an empty list must hide the segment from the line loop: {bare_line}"
    );
    assert_eq!(
        status_segments(&bare).task_summary,
        None,
        "an empty list must hide the segment from the TUI view"
    );
}

/// The TUI bar carries the same `tasks:` words the headless header renders —
/// through the bar's own word seam, not by re-reading the headless line.
#[test]
fn the_tui_bar_carries_the_same_tasks_words() {
    use crate::interactive::session_prompt::status_segments;
    use crate::interactive::session_state::SessionState;
    use crate::interactive::tui::ui::chrome::status::status_words_for_test;

    let mut state = SessionState::new("s1", Some(String::from("analytics")), "m");
    state.task_list = SessionTaskList::new(vec![
        SessionTask::new("one", TaskStatus::Done).unwrap(),
        SessionTask::new("two", TaskStatus::Pending).unwrap(),
    ])
    .unwrap();
    let view = status_segments(&state);
    assert!(
        status_words_for_test(&view).contains("tasks: 1/2"),
        "the TUI bar must carry the same words as the header"
    );

    let bare = SessionState::new("s1", Some(String::from("analytics")), "m");
    let bare_view = status_segments(&bare);
    assert!(
        !status_words_for_test(&bare_view).contains("tasks:"),
        "an empty list must hide the segment from the TUI bar too"
    );
}
