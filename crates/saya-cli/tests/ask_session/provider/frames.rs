// The wire bodies one row scripts, spelled exactly as the provider sends them:
// one SSE reply per round, ended by the stream terminator.

/// One SSE reply that speaks `text` and ends the stream.
pub(crate) fn answer_body(text: &str) -> String {
    let chunk = serde_json::json!({
        "choices": [{ "delta": { "content": text } }],
    });
    let done = format!("data: [{}]\n\n", "DONE");
    format!("data: {chunk}\n\n{done}")
}

/// One SSE reply that calls `name` with `arguments` and ends the stream.
pub(crate) fn tool_body(name: &str, arguments: serde_json::Value) -> String {
    let chunk = serde_json::json!({
        "choices": [{
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "id": format!("call-{name}"),
                    "function": {
                        "name": name,
                        "arguments": arguments.to_string(),
                    },
                }],
            },
            "finish_reason": "tool_calls",
        }],
    });
    let done = format!("data: [{}]\n\n", "DONE");
    format!("data: {chunk}\n\n{done}")
}

/// The question the scripted model asks instead of assuming, as the wire
/// spells it.
pub(crate) fn clarify_body(question: &str, options: [&str; 2]) -> String {
    tool_body(
        "request_clarification",
        serde_json::json!({ "question": question, "options": options }),
    )
}
