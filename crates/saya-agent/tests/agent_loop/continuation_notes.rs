/// Counts user-role messages in `messages` carrying the continuation note,
/// keyed on its fixed wording rather than on the private constant.
fn continuation_notes(messages: &[ChatMessage]) -> usize {
    messages
        .iter()
        .filter(|message| message.role == "user" && message.content.contains("output-token limit"))
        .count()
}
