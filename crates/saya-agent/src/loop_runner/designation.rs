//! Reads the optional `designate_answer` call by which the model names the SQL
//! that answers the question at the terminal turn.

use crate::DESIGNATE_ANSWER_TOOL;

/// The SQL the model designated as the answering query, when the terminal turn
/// contains a `designate_answer` call whose `sql` argument is a string. `None`
/// for a turn without the call (or a malformed argument) so the protocol stays
/// optional.
pub(super) fn designation_from(assistant: &crate::ChatMessage) -> Option<String> {
    assistant.tool_calls.iter().find_map(|call| {
        if call.name == DESIGNATE_ANSWER_TOOL {
            call.arguments
                .get("sql")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        } else {
            None
        }
    })
}
