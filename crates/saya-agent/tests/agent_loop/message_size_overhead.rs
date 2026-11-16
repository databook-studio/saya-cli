/// Approximates the loop's `message_size` for assertion purposes (content plus
/// role plus tool-call argument bytes), matching what the bound measures.
fn message_size_overhead(message: &ChatMessage) -> usize {
    message
        .tool_calls
        .iter()
        .map(|call| {
            call.id.len()
                + call.name.len()
                + serde_json::to_string(&call.arguments).map_or(0, |text| text.len())
        })
        .sum::<usize>()
}
