use super::*;

// ---------------------------------------------------------------------------
// The note bound: at most MAX_TASK_NOTE_CHARS
// ---------------------------------------------------------------------------

#[test]
fn the_note_admits_exactly_max_chars_and_refuses_one_more() {
    let max = "n".repeat(MAX_TASK_NOTE_CHARS);
    assert_eq!(max.chars().count(), 512, "the bound pins the scale");
    noted("task", TaskStatus::Pending, &max);

    let over = "n".repeat(MAX_TASK_NOTE_CHARS + 1);
    let error = SessionTask::with_note("task", TaskStatus::Pending, Some(&over)).unwrap_err();
    assert!(
        matches!(error, SessionTaskError::NoteTooLong),
        "one past the bound refuses: {error:?}"
    );
}
