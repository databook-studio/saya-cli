/// One SSE `data:` frame carrying a single tool call, ready for the mock to
/// serve. Bodies are built with `serde_json`, never by hand-escaping.
pub(crate) fn tool_call_body(call_id: &str, name: &str, args: serde_json::Value) -> String {
    let chunk = serde_json::json!({
        "choices": [{
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "id": call_id,
                    "function": {
                        "name": name,
                        "arguments": serde_json::to_string(&args).unwrap(),
                    },
                }],
            },
        }],
    });
    format!("data: {chunk}\n\ndata: [DONE]\n\n")
}

/// One SSE `data:` frame carrying a plain-text final answer.
pub(crate) fn text_body(text: &str) -> String {
    let chunk = serde_json::json!({
        "choices": [{
            "delta": { "content": text },
        }],
    });
    format!("data: {chunk}\n\ndata: [DONE]\n\n")
}
