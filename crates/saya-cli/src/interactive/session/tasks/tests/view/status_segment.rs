//! The `tasks:` status segment: what the headless header counts, and the
//! anti-drift agreement between the line loop, the TUI view, and the TUI bar.

use crate::interactive::session::tasks::view::tasks_summary;
use saya_types::{SessionTask, SessionTaskList, TaskStatus};

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
    use crate::interactive::session::state::SessionState;
    use crate::interactive::session_prompt::{status_line, status_segments};

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
    use crate::interactive::session::state::SessionState;
    use crate::interactive::session_prompt::status_segments;
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
