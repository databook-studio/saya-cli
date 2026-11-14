use super::mock::request_json;

/// The rows the model saw: the tool-result message of the follow-up request.
pub(crate) fn tool_result_rows(request_body: &str) -> Vec<Vec<serde_json::Value>> {
    let body = request_json(request_body);
    let messages = body.get("messages").and_then(|m| m.as_array()).unwrap();
    let tool_message = messages
        .iter()
        .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("tool"))
        .expect("the follow-up request carries the tool result");
    let content = tool_message
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap();
    let result: serde_json::Value = serde_json::from_str(content).unwrap();
    result
        .get("rows")
        .and_then(|rows| rows.as_array())
        .unwrap()
        .iter()
        .map(|row| row.as_array().unwrap().clone())
        .collect()
}
