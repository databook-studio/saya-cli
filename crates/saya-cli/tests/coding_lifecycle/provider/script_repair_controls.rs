use super::{answer, tagged_tool};
use serde_json::json;
pub(super) fn response(actor: &str, stage: usize, request: &str) -> String {
    if actor != "parent" {
        return super::repair_continuation::response(actor, stage, request);
    }
    match stage {
        0 => tagged_tool(
            "control-join",
            "helpers",
            json!({"description":"prepare the owned file","prompt":"write for child-one; prepare the owned file","subagent_type":"general","agent_id":"child-one"}),
        ),
        _ => answer("owned child completed; await explicit operator continuation"),
    }
}
