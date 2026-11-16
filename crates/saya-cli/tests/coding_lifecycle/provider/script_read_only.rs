//! Mode 118: a read-only job reads `tracked.txt` and answers from the result.
//! The first turn asks for the file; every later turn answers with the content
//! the tool returned, so the test can prove the read reached the model.

use serde_json::{Value, json};

use super::render::{answer, tool};

pub(super) fn response(actor: &str, stage: usize, request: &str) -> String {
    if actor != "parent" {
        return answer("unexpected child");
    }
    match stage {
        0 => tool("read", json!({"path": "tracked.txt"})),
        _ => answer(&read_answer(request)),
    }
}

fn read_answer(request: &str) -> String {
    let Some(content) = last_tool_content(request) else {
        return "the file could not be read".into();
    };
    format!("the file reads: {content}")
}

/// The content of the most recent tool result in the request, when the result
/// is a workspace read.
fn last_tool_content(request: &str) -> Option<String> {
    let request: Value = serde_json::from_str(request).ok()?;
    request["messages"]
        .as_array()?
        .iter()
        .rev()
        .find_map(|message| {
            if message["role"].as_str() != Some("tool") {
                return None;
            }
            let content = message["content"].as_str()?;
            let result: Value = serde_json::from_str(content).ok()?;
            result["content"].as_str().map(str::to_owned)
        })
}
