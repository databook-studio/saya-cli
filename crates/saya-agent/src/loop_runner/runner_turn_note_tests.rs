//! The session-tasks note rides the end of the transcript: a new note is
//! appended, an unchanged one is not appended twice, and nothing already sent
//! is rewritten.

use super::refresh_turn_note;
use crate::ChatMessage;

#[test]
fn a_later_note_is_appended_and_leaves_the_prefix_byte_identical() {
    let mut messages = vec![
        ChatMessage::text("user", "source: session-tasks\nold"),
        ChatMessage::text("assistant", "work"),
    ];
    let prefix = messages.clone();
    refresh_turn_note(&mut messages, Some("source: session-tasks\nnew".into()));
    assert_eq!(
        messages[..prefix.len()],
        prefix[..],
        "the prefix already sent to the provider is untouched"
    );
    assert_eq!(messages.len(), prefix.len() + 1);
    assert_eq!(
        messages.last().unwrap().content,
        "source: session-tasks\nnew"
    );
}

#[test]
fn an_unchanged_note_is_not_appended_twice() {
    let mut messages = vec![ChatMessage::text("user", "source: session-tasks\nsame")];
    refresh_turn_note(&mut messages, Some("source: session-tasks\nsame".into()));
    assert_eq!(messages.len(), 1);
}
