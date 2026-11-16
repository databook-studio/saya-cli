//! Actual observed digests select a single owned native deletion.
use super::render::{answer, tagged_tool};
use serde_json::{Value, json};

pub(super) fn response(actor: &str, stage: usize, request: &str) -> String {
    if actor == "parent" {
        return match stage {
            // One `helpers` call per turn: a call waits for its helper, so the
            // second child is admitted only after the first has finished.
            0 => helper(
                "call-task-a",
                "delete the owned file",
                "child-a",
                "write for child-a: read the owned file, refuse a stale deletion then delete using the actual observed digest",
            ),
            1 => helper(
                "call-task-b",
                "finish without making changes",
                "child-b",
                "write for child-b: finish without making changes",
            ),
            // Direct mode: the owner prepares at the end of the run from the
            // shared working checkout, which carries the child's confirmed
            // deletion.
            _ => answer("Joined and reviewed actual deletion without applying to the original"),
        };
    }
    if actor != "child-a" {
        return answer("No workspace effect was requested from this child");
    }
    match stage {
        0 => tagged_tool("delete-observed", "read", json!({"path":"tracked.txt"})),
        1 => tagged_tool(
            "delete-stale",
            "delete",
            json!({"path":"tracked.txt","expected_digest":"0".repeat(64)}),
        ),
        2 => tagged_tool("delete-after-stale", "read", json!({"path":"tracked.txt"})),
        3 => tagged_tool(
            "delete-confirmed",
            "delete",
            json!({"path":"tracked.txt","expected_digest":observed_digest(request)}),
        ),
        4 => tagged_tool(
            "delete-after-confirmed",
            "read",
            json!({"path":"tracked.txt"}),
        ),
        _ => answer("Actual guarded deletion result and subsequent read were observed"),
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

fn observed_digest(request: &str) -> Option<String> {
    let request: Value = serde_json::from_str(request).ok()?;
    let message = request["messages"]
        .as_array()?
        .iter()
        .rev()
        .find(|message| message["tool_call_id"] == "delete-after-stale")?;
    let result: Value = serde_json::from_str(message["content"].as_str()?).ok()?;
    result["digest"].as_str().map(str::to_owned)
}
