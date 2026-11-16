use serde_json::Value;

/// The connection name the job's consent selected, read from the advertised
/// `schema_discovery` tool: the coding surface narrows that tool's `connection`
/// enum to the admitted profiles, so its first choice is the one the job may
/// name.
pub(super) fn selected_connection(request: &str) -> String {
    let body: Value = serde_json::from_str(request).unwrap();
    body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["function"]["name"] == "schema_discovery")
        .and_then(|tool| {
            tool["function"]["parameters"]["properties"]["connection"]["enum"][0].as_str()
        })
        .unwrap_or("missing-connection-choice")
        .to_owned()
}

pub(crate) fn workspace_digest(request: &str) -> Option<String> {
    let body: Value = serde_json::from_str(request).ok()?;
    let read = body["messages"]
        .as_array()?
        .iter()
        .rev()
        .find(|message| message["tool_call_id"] == "call-read")?;
    let result: Value = serde_json::from_str(read["content"].as_str()?).ok()?;
    result["digest"].as_str().map(str::to_owned)
}
