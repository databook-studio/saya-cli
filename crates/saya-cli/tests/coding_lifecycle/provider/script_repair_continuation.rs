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
        // The retired `coding_review` and `follow_up_child` calls have no
        // replacement: the owner prepares at the end of the run, and the
        // operator drives the follow-up over the resumed job, so the root's
        // remaining turns are plain reads of the shared checkout.
        return match stage {
            0 => tagged_tool(
                "first-join",
                "helpers",
                json!({"description":"prepare the first owned candidate","prompt":"write for child-one; prepare the first owned candidate","subagent_type":"general","agent_id":"child-one"}),
            ),
            1 => tagged_tool("first-review", "read", json!({"path":"tracked.txt"})),
            2 => tagged_tool("follow-up", "read", json!({"path":"tracked.txt"})),
            3 => tagged_tool("second-join", "read", json!({"path":"tracked.txt"})),
            4 => tagged_tool("second-replay", "read", json!({"path":"tracked.txt"})),
            5 => tagged_tool("second-review", "read", json!({"path":"tracked.txt"})),
            6 => tagged_tool(
                "second-delegation",
                "helpers",
                json!({"description":"write tracked.txt","prompt":"write for child-extra","subagent_type":"general","agent_id":"child-extra"}),
            ),
            7 => tagged_tool("resumed-replay", "read", json!({"path":"tracked.txt"})),
            _ => answer("repaired and reviewed the same owned child without apply"),
        };
    }
    match stage {
        // The first pass only inspects the owned path: the child's delta is
        // empty, so its work is NOT in the root working checkout W. The operator
        // follow-up these tests drive is the follow-up of an UNMERGED child; a
        // child whose delta is in W is refused (`CodingError::ChildAlreadyMerged`).
        0 => tagged_tool("initial-read", "read", json!({"path":"tracked.txt"})),
        1 => answer("child-one inspected the owned path without changing it"),
        2 => tagged_tool("repair-read", "read", json!({"path":"tracked.txt"})),
        3 => tagged_tool(
            "repair-write",
            "write",
            json!({"path":"tracked.txt","content":"FIXED\n",
            "expected_digest":result(request, "repair-read")["digest"]}),
        ),
        _ => answer("owned phase complete"),
    }
}
