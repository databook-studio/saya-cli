use crate::ChatMessage;

/// Appends the session-tasks note after everything already sent. A note is
/// never removed: removing an earlier one would rewrite the prefix a provider
/// has already cached, so the list is append-only and a changed list rides the
/// end of the user turn.
pub(super) fn refresh_turn_note(messages: &mut Vec<ChatMessage>, note: Option<String>) {
    let Some(note) = note.filter(|note| !note.is_empty()) else {
        return;
    };
    if messages
        .last()
        .is_some_and(|message| message.role == "user" && message.content == note)
    {
        return;
    }
    messages.push(ChatMessage::text("user", note));
}

#[cfg(test)]
#[path = "../runner_turn_note_tests.rs"]
mod turn_note_tests;
