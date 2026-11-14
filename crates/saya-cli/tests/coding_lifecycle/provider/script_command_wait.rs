use super::render::{answer, tool};
use serde_json::json;

pub(super) fn response(actor: &str, stage: usize, _request: &str) -> String {
    if actor == "parent" {
        return match stage {
            0 => tool(
                "helpers",
                json!({"description":"run the long command","prompt":"write for child-a","subagent_type":"general","agent_id":"child-a"}),
            ),
            _ => answer("native command receipt sequence completed"),
        };
    }
    // A command is one synchronous `shell` call: the command runs to completion
    // inside it, and the sandbox keeps a headless job from asking for host
    // approval.
    let round = stage / 4;
    if round < 5 {
        return match stage % 4 {
            0 => tool(
                "shell",
                json!({"command":"sleep 1","description":"run the long command","sandbox":true}),
            ),
            _ => tool(
                "shell",
                json!({"command":"true","description":"observe the command","sandbox":true}),
            ),
        };
    }
    match stage {
        20 => tool(
            "shell",
            json!({"command":"true","description":"reobserve a retired command","sandbox":true}),
        ),
        21 => tool(
            "shell",
            json!({"command":"sleep 1","description":"run another long command","sandbox":true}),
        ),
        22 => tool(
            "shell",
            json!({"command":"true","description":"observe the fresh command","sandbox":true}),
        ),
        _ => answer("receipt retention allowed a fresh command after stale-id refusal"),
    }
}
