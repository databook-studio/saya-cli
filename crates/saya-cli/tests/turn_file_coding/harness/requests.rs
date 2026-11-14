/// The parsed request body of one captured raw request.
pub fn request_json(raw: &str) -> serde_json::Value {
    let (_, body) = raw.split_once("\r\n\r\n").expect("HTTP request has a body");
    serde_json::from_str(body).expect("request body parses as JSON")
}

/// The tool names the request advertises (`tools[].function.name`).
pub fn tool_names(request: &serde_json::Value) -> Vec<&str> {
    request["tools"]
        .as_array()
        .expect("request advertises tools")
        .iter()
        .map(|tool| {
            tool["function"]["name"]
                .as_str()
                .expect("tool names its function")
        })
        .collect()
}

/// The model-facing tool results in the request, in conversation order.
pub fn tool_results(request: &serde_json::Value) -> Vec<&str> {
    request["messages"]
        .as_array()
        .expect("request carries messages")
        .iter()
        .filter(|message| message["role"] == "tool")
        .filter_map(|message| message["content"].as_str())
        .collect()
}

/// The last tool-result message of a captured request: the model-facing
/// result of the call the previous round made.
pub fn last_tool_result(raw: &str) -> String {
    tool_results(&request_json(raw))
        .last()
        .copied()
        .expect("the request carries a tool result")
        .to_owned()
}

/// The `digest` the read tool reported: parsed from the tool-result message
/// in the request body — the read's JSON result carries `digest` (a plain
/// string scan cannot work: the tool descriptions themselves say "digest").
pub fn digest_from(raw: &str) -> String {
    for content in tool_results(&request_json(raw)) {
        if let Some(at) = content.find("\"digest\":\"") {
            let hex: String = content[at + 10..]
                .chars()
                .take_while(char::is_ascii_hexdigit)
                .collect();
            if hex.len() == 64 {
                return hex;
            }
        }
    }
    panic!(
        "no tool result in the request carries a digest: {:?}",
        tool_results(&request_json(raw))
    );
}

pub fn stdout(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}
