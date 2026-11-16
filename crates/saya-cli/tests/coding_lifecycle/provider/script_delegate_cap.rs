use super::render::{answer, tagged_tool};
use serde_json::json;

/// The model-facing assignment cap the `helpers` tool advertises
/// (`task_flow::MAX_HELPER_ASSIGNMENT_BYTES`): a call's assignment is
/// `description\n\nprompt`, so the tool admits exactly this many bytes and
/// refuses one byte more.
const TOOL_CAP: usize = 32 * 1024;

/// A prompt of exactly `bytes` bytes whose text carries `write for <agent>`, so
/// the transport attributes the child's turns to that agent.
fn prompt(agent: &str, bytes: usize) -> String {
    let lead = format!("write for {agent}; ");
    format!("{lead}{}", "x".repeat(bytes - lead.len()))
}

/// The parent offers one assignment a byte over the tool's cap, then two
/// assignments at the full cap, one `helpers` call per turn: a call waits for
/// its helper, so a second call in the same turn could only be refused as busy.
pub(super) fn response(actor: &str, stage: usize) -> String {
    if actor == "parent" {
        return match stage {
            0 => helper("over-cap", 0, "child-over", TOOL_CAP + 1),
            1 => helper("admitted", 0, "child-a", TOOL_CAP),
            2 => helper("admitted", 1, "child-b", TOOL_CAP),
            _ => answer("the full-cap assignments were admitted"),
        };
    }
    match stage {
        0 => tagged_tool(
            "write",
            "write",
            json!({"path":if actor == "child-a" { "new-a.txt" } else { "new-b.txt" },"content":format!("native {actor}\n")}),
        ),
        _ => answer("child work complete"),
    }
}

/// One `helpers` call whose assignment is exactly `bytes` bytes: the
/// description, the blank line, and a prompt sized to fill the rest.
fn helper(tag: &str, index: usize, agent: &str, bytes: usize) -> String {
    let description = format!("hand piece {index} to a helper");
    let prompt = prompt(agent, bytes - description.len() - 2);
    tagged_tool(
        &format!("{tag}-{index}"),
        "helpers",
        json!({
            "description": description,
            "prompt": prompt,
            "subagent_type": "general",
            "agent_id": agent,
        }),
    )
}
