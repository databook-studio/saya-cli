//! The shared dispatcher's `/tasks` surface: the list's content the model
//! writes, the exact view `/tasks` answers with, and what `/tasks clear`
//! empties and stops injecting.

use saya_types::{SessionTask, SessionTaskList, TaskStatus};

/// The shared dispatcher shows the list: `/tasks` on an empty session names
/// the absence, on a mixed list renders the exact view, and on a fully-done
/// list reads full.
#[test]
fn tasks_through_apply_renders_empty_mixed_and_done() {
    use crate::interactive::SessionAction;
    use crate::interactive::session::state::SessionState;
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
    use crate::interactive::session::state::SessionState;
    use crate::interactive::session::tasks::render::render_tasks_block;
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
