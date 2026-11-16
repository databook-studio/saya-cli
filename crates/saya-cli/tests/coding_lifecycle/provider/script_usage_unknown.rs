use super::{answer, tagged_tool};
use serde_json::{Value, json};

fn result(request: &str, id: &str) -> Value {
    let request: Value = serde_json::from_str(request).unwrap();
    request["messages"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|message| message["tool_call_id"] == id)
        .and_then(|message| serde_json::from_str(message["content"].as_str()?).ok())
        .unwrap_or(Value::Null)
}

pub(super) fn response(actor: &str, stage: usize, request: &str) -> String {
    if actor == "parent" {
        // Direct mode advertises no `follow_up_child` root tool: the root hands
        // its piece to one helper and the test drives the operator follow-up
        // over the resumed job (the surviving follow-up surface). The call keeps
        // the `first-join` id the tests read the result under.
        return match stage {
            0 => tagged_tool(
                "first-join",
                "helpers",
                json!({"description":"write tracked.txt","prompt":"write for child-one; first pass","subagent_type":"general","agent_id":"child-one"}),
            ),
            _ => answer("the child recovered from a cut stream"),
        };
    }
    match stage {
        // The first pass only reads the owned path: the child's delta is empty,
        // so its work is NOT in the root working checkout W. The operator
        // follow-up below is the follow-up of an UNMERGED child; a child whose
        // delta is in W is refused (`CodingError::ChildAlreadyMerged`).
        0 => tagged_tool("initial-read", "read", json!({"path":"tracked.txt"})),
        1 => answer("child-one inspected the owned path without changing it"),
        2 => tagged_tool("second-read", "read", json!({"path":"tracked.txt"})),
        3 => tagged_tool(
            "second-write",
            "write",
            json!({"path":"tracked.txt","content":"TWO\n",
            "expected_digest":result(request, "second-read")["digest"]}),
        ),
        _ => answer("owned phase complete"),
    }
}

/// The parent hands one piece to a helper; the child never gets a real response
/// (every stream it receives is cut), so it only matters that the parent answers
/// once the failed helper is summarized.
pub(super) fn exhausted(actor: &str, stage: usize) -> String {
    if actor != "parent" {
        return answer("unreachable: every child stream is cut");
    }
    match stage {
        0 => tagged_tool(
            "first-join",
            "helpers",
            json!({"description":"write tracked.txt","prompt":"write for child-one; first pass","subagent_type":"general","agent_id":"child-one"}),
        ),
        _ => answer("the child's provider requests were exhausted"),
    }
}
