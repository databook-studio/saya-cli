use super::render::{answer, tagged_tool};
use serde_json::json;

pub(super) fn response(actor: &str, stage: usize) -> String {
    if actor == "parent" {
        return match stage {
            // One `helpers` call per turn: a call waits for its helper, so the
            // second child is admitted only after the first has finished.
            0 => helper("call-task-a", "write new-a.txt", "child-a"),
            1 => helper("call-task-b", "write new-b.txt", "child-b"),
            _ => answer("created, reread, assembled and reviewed owned changes"),
        };
    }
    let path = if actor == "child-a" {
        "new-a.txt"
    } else {
        "new-b.txt"
    };
    match stage {
        0 => tagged_tool(&format!("missing-{actor}"), "read", json!({"path":path})),
        1 => tagged_tool(
            &format!("create-{actor}"),
            "write",
            json!({"path":path,"content":format!("created {actor}\n")}),
        ),
        2 => tagged_tool(&format!("reread-{actor}"), "read", json!({"path":path})),
        _ => answer("owned file read back after real creation"),
    }
}

/// One `helpers` call: the child's own task text carries `write for <agent>`,
/// which is how the transport attributes the child's turns to that agent.
fn helper(id: &str, description: &str, agent: &str) -> String {
    tagged_tool(
        id,
        "helpers",
        json!({
            "description": description,
            "prompt": format!("write for {agent}"),
            "subagent_type": "general",
            "agent_id": agent,
        }),
    )
}
