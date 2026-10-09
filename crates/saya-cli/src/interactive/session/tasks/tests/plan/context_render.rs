use super::*;

/// A non-empty list produces exactly one context block on the user turn; an
/// empty list produces none.
#[test]
fn the_list_rides_the_user_turn_only_when_non_empty() {
    use saya_types::{SessionTask, SessionTaskList, TaskStatus};
    let empty = SessionTaskList::default();
    assert!(
        render_tasks_block(&empty).is_none(),
        "an empty list costs zero tokens and emits nothing"
    );
    let list = SessionTaskList::new(vec![
        SessionTask::with_note(
            "profile the tables",
            TaskStatus::InProgress,
            Some("halfway"),
        )
        .unwrap(),
        SessionTask::new("write the report", TaskStatus::Pending).unwrap(),
    ])
    .unwrap();
    let block = render_tasks_block(&list).expect("a non-empty list emits one block");
    assert_eq!(block.label, TASKS_BLOCK_LABEL);
    assert!(!block.truncated);
    let rendered = saya_agent::render_untrusted_block(&block);
    assert!(rendered.contains("profile the tables"));
    assert!(rendered.contains("write the report"));
}

/// The system prompt is byte-identical whether or not a list exists — the
/// prefix-cache property. Asserted through the same `build_messages` the
/// runtime uses: the system message must not vary with the list.
#[test]
fn the_system_prompt_is_byte_identical_with_or_without_a_list() {
    use saya_types::{SessionTask, SessionTaskList, TaskStatus};
    let list = SessionTaskList::new(vec![
        SessionTask::new("profile the tables", TaskStatus::InProgress).unwrap(),
    ])
    .unwrap();
    let with = render_tasks_block(&list).map(|block| vec![block]);
    for blocks in [None, with.as_deref()] {
        let _ = blocks;
    }
    let empty: Vec<saya_agent::ContextBlock> = Vec::new();
    let full: Vec<saya_agent::ContextBlock> = render_tasks_block(&list).into_iter().collect();
    let system = Some("Available database connections:\n- a (postgresql)");
    let a = saya_agent::build_messages(system, &empty, "what next", &[], 32 * 1024).unwrap();
    let b = saya_agent::build_messages(system, &full, "what next", &[], 32 * 1024).unwrap();
    assert_eq!(
        a[0], b[0],
        "the system message must not vary with the task list — that is what makes the prefix cache work"
    );
    assert_ne!(a[1], b[1], "the user turns differ: one carries the list");
}

/// The rendered block's size is bounded by the contract's own bounds: a
/// worst-case list (32 tasks, max-length titles and notes) renders without
/// exceeding the stated ceiling.
#[test]
fn the_worst_case_list_renders_within_the_stated_ceiling() {
    use saya_types::{
        MAX_SESSION_TASKS, MAX_TASK_NOTE_CHARS, MAX_TASK_TITLE_CHARS, SessionTask, SessionTaskList,
        TaskStatus,
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
    let block = render_tasks_block(&list).expect("worst case still renders one block");
    let rendered = saya_agent::render_untrusted_block(&block);
    assert!(
        rendered.len() <= TASKS_RENDER_CEILING,
        "worst-case render {} exceeds ceiling {TASKS_RENDER_CEILING}",
        rendered.len()
    );
    // The ceiling is stated here, not just in the source: 32 tasks ×
    // (200-char title + 512-char note + markers) plus the wrapper.
    assert_eq!(
        TASKS_RENDER_CEILING, 24_576,
        "the ceiling pins the scale: {TASKS_RENDER_CEILING}"
    );
}

/// Task validation limits Unicode scalar values; rendering must separately
/// keep the escaped context block within its byte budget and disclose cuts.
#[test]
fn multibyte_task_text_is_byte_bounded_and_flagged_as_truncated() {
    use saya_types::{
        MAX_SESSION_TASKS, MAX_TASK_NOTE_CHARS, MAX_TASK_TITLE_CHARS, SessionTask, SessionTaskList,
        TaskStatus,
    };
    let title = "🧪".repeat(MAX_TASK_TITLE_CHARS);
    let note = format!("{}\\", "🧪".repeat(MAX_TASK_NOTE_CHARS - 1));
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
    let block = render_tasks_block(&list).unwrap();
    let rendered = saya_agent::render_untrusted_block(&block);
    assert!(
        rendered.len() <= TASKS_RENDER_CEILING,
        "{} bytes exceeds {TASKS_RENDER_CEILING}",
        rendered.len()
    );
    assert!(
        block.truncated,
        "omitted task content must set the ContextBlock truncation flag"
    );
    assert!(rendered.contains(TASKS_BLOCK_LABEL));
    assert!(rendered.contains("[active]"));
    assert!(rendered.contains("\\\\"), "task backslashes remain escaped");
    assert!(rendered.contains("[truncated:"));
}

/// The live cell round-trips through render: what `tasks_set` stored is
/// what the next turn's block carries.
#[test]
fn the_cell_round_trips_through_render() {
    use saya_types::{SessionTask, SessionTaskList, TaskStatus};
    let tasks = SessionTasks::default();
    assert!(
        render_tasks_block(&tasks.current()).is_none(),
        "a fresh cell renders nothing"
    );
    tasks.replace(
        SessionTaskList::new(vec![
            SessionTask::new("profile the tables", TaskStatus::Done).unwrap(),
        ])
        .unwrap(),
    );
    let block = render_tasks_block(&tasks.current()).expect("stored renders");
    assert!(
        saya_agent::render_untrusted_block(&block).contains("profile the tables"),
        "the stored list is what the turn carries"
    );
}
