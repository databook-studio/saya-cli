//! What a tool result the model received is, as a golden records it.
//!
//! A record never keeps a result's text — it keeps the class: `ok`,
//! `refused-by-safety`, or `error`. The classification lives here, apart from
//! the record itself, so the record's own file stays one concern.

use std::collections::HashMap;

/// Files a tool result under its call id, for the class the golden states.
pub(crate) fn record_tool_result(
    message: &serde_json::Value,
    results: &mut HashMap<String, String>,
) {
    if message["role"] != "tool" {
        return;
    }
    if let (Some(id), Some(content)) = (
        message["tool_call_id"].as_str(),
        message["content"].as_str(),
    ) {
        results.insert(id.to_owned(), content.to_owned());
    }
}

/// Classifies a tool result the model received. A safety refusal is named by
/// the one policy phrase the safety layer emits; any other error is `error`.
pub(crate) fn classify_result(content: &str) -> &'static str {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(content) {
        if let Some(error) = value.get("error").and_then(serde_json::Value::as_str) {
            return classify_error(error);
        }
        return "ok";
    }
    classify_error(content)
}

fn classify_error(text: &str) -> &'static str {
    if text.contains("query rejected by read-only safety policy") {
        "refused-by-safety"
    } else {
        "error"
    }
}
