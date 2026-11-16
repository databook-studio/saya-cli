use super::render::{answer, tagged_tool};
use serde_json::{Value, json};
fn digest(request: &str, id: &str) -> Option<String> {
    let body: Value = serde_json::from_str(request).ok()?;
    let message = body["messages"]
        .as_array()?
        .iter()
        .rev()
        .find(|message| message["tool_call_id"] == id)?;
    let result: Value = serde_json::from_str(message["content"].as_str()?).ok()?;
    result["digest"].as_str().map(str::to_owned)
}
pub(super) fn response(actor: &str, stage: usize, request: &str) -> String {
    if actor == "parent" {
        return match stage {
            // One `helpers` call per turn: a call waits for its helper, so the
            // second child is admitted only after the first has finished.
            0 => helper(
                "call-task-a",
                "exercise bounded deletion refusals",
                "child-a",
                "write for child-a: exercise bounded deletion refusals and retain every file",
            ),
            1 => helper(
                "call-task-b",
                "finish without mutation",
                "child-b",
                "write for child-b: finish without mutation",
            ),
            _ => answer("Known deletion refusals observed without apply"),
        };
    }
    if actor != "child-a" {
        return answer("No mutation requested");
    }
    match stage {
        0 => tagged_tool("delete-observed", "read", json!({"path":"tracked.txt"})),
        1 => tagged_tool(
            "delete-missing-digest",
            "delete",
            json!({"path":"tracked.txt"}),
        ),
        2 => tagged_tool(
            "delete-null-digest",
            "delete",
            json!({"path":"tracked.txt","expected_digest":null}),
        ),
        3 => tagged_tool(
            "delete-uppercase",
            "delete",
            json!({"path":"tracked.txt","expected_digest":digest(request,"delete-observed").map(|value|value.to_uppercase())}),
        ),
        4 => tagged_tool(
            "delete-extra-property",
            "delete",
            json!({"path":"tracked.txt","expected_digest":digest(request,"delete-observed"),"recursive":true}),
        ),
        5 => tagged_tool(
            "delete-private-create",
            "write",
            json!({"path":".env","content":"ordinary synthetic private file\n"}),
        ),
        6 => tagged_tool(
            "delete-private",
            "delete",
            json!({"path":".env","expected_digest":digest(request,"delete-private-create")}),
        ),
        7 => tagged_tool(
            "delete-ignored-create",
            "write",
            json!({"path":"generated.tmp","content":"generated source\n"}),
        ),
        8 => tagged_tool(
            "delete-ignored",
            "delete",
            json!({"path":"generated.tmp","expected_digest":digest(request,"delete-ignored-create")}),
        ),
        9 => tagged_tool("delete-retained", "read", json!({"path":"tracked.txt"})),
        _ => answer("All targets remain present after strict deletion refusals"),
    }
}

/// One `helpers` call: the prompt carries `write for <agent>`, which is how the
/// transport attributes the child's turns to that agent.
fn helper(id: &str, description: &str, agent: &str, prompt: &str) -> String {
    tagged_tool(
        id,
        "helpers",
        json!({
            "description": description,
            "prompt": prompt,
            "subagent_type": "general",
            "agent_id": agent,
        }),
    )
}
