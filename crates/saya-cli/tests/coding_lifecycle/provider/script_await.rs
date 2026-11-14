//! Scripted provider responses for the coordinator family: the
//! unregistered-child refusal probe, the durable helper replay, and the empty
//! helper call.

use super::{answer, tagged_tool, tool};
use serde_json::json;

/// The parent scripts for modes 37 (coordinator refusals), 38 (helper replay),
/// and 39 (empty helper call); the dispatcher calls this only for those modes.
pub(super) fn response(mode: u8, stage: usize) -> String {
    match mode {
        37 => match stage {
            0 => tool(
                "helpers",
                json!({"description":"outside assignment","prompt":"write for child-a","subagent_type":"general","agent_id":"child-a"}),
            ),
            // The retired `message_child` / `cancel_child` calls have no
            // replacement: `helpers` is the root's only coordinator tool.
            1 => tagged_tool("unknown-message", "read", json!({"path":"tracked.txt"})),
            2 => tagged_tool("unknown-cancel", "read", json!({"path":"tracked.txt"})),
            _ => answer("known coordinator refusals were corrected or reported"),
        },
        38 => match stage {
            0 => tagged_tool(
                "await-first",
                "helpers",
                json!({"description":"write new-a.txt","prompt":"write for child-a","subagent_type":"general","agent_id":"child-a"}),
            ),
            1 => tagged_tool(
                "await-second",
                "helpers",
                json!({"description":"write new-b.txt","prompt":"write for child-b","subagent_type":"general","agent_id":"child-b"}),
            ),
            _ => answer("durable child completions were replayed"),
        },
        _ => match stage {
            // No tool remains that means "wait for nothing": a `helpers` call
            // always names one helper, so the empty await is a plain read.
            0 => tagged_tool("await-empty", "read", json!({"path":"tracked.txt"})),
            _ => answer("no children were admitted"),
        },
    }
}
